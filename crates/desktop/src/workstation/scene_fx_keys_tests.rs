//! F057/F058 (P3): the animation controls of the "Efeitos" section — the switch of a boolean
//! parameter, the seed of a noise effect and the keys of every parameter — driven through the
//! semantic nodes of the clip and by real GPUI clicks over the buttons that paint them.
//!
//! The rules (the keys, their order, the easings, the sampling, the seed, the limits) are the
//! core's and are tested in `crates/core/tests/scene_fx_keys.rs`; these tests prove the window
//! reaches them: one op and one undo entry per gesture, the typed refusal on screen, the
//! disabled controls with their reason, nothing keyed by moving a slider, and a node surface that
//! says what the panel shows. Not proved here: pixels in the monitor (see
//! `crates/render/tests/scene_fx.rs` and `runtime/test/directed-scene-grain-browser.test.mjs`).
use super::scene_fx_tests::{
    center_of, clip_state, clips, field_value, invoke, mounted, node, set, undo_depth,
};
use super::*;
// The Kit's test macro expands to `#[test]`; the glob import above would resolve it to the Kit macro again.
use core::prelude::v1::test;

const GRAIN: &str = "scene.fx.clip:fx-1";

fn add_grain(view: &Entity<Workstation>, cx: &mut VisualTestContext) {
    invoke(view, cx, "scene.fx", "add:filmGrain").unwrap();
}

fn animation(view: &Entity<Workstation>, cx: &mut VisualTestContext, param: &str) -> Value {
    clip_state(view, cx, "fx-1")["animation"]["params"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["param"] == param)
        .unwrap_or_else(|| panic!("no animation for `{param}`"))
        .clone()
}

fn action(node: &Value, name: &str) -> Value {
    node["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["action"] == name)
        .unwrap_or_else(|| panic!("no action `{name}` on {}", node["id"]))
        .clone()
}

fn move_playhead_to_frame(view: &Entity<Workstation>, cx: &mut VisualTestContext, frame: i64) {
    // The NLE playhead is in sequence frames; the section's `playhead_frame` is the scene's own.
    // Walk the playhead until the section reports `frame`.
    for candidate in 0..400 {
        view.update(cx, |view, _| view.state.timeline.set_playhead(candidate));
        let here = node(view, cx, "scene.fx")["state"]["playhead_frame"].as_i64();
        if here == Some(frame) {
            return;
        }
    }
    panic!("no playhead position shows frame {frame}");
}

#[gpui_kit::test]
fn a_switch_is_a_boolean_node_that_takes_true_or_false_as_one_entry(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    add_grain(&view, cx);
    let depth = undo_depth(&view, cx);
    let clip = node(&view, cx, GRAIN);
    let colored = clip["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["field"] == "colored")
        .unwrap()
        .clone();
    assert_eq!(colored["type"], json!("boolean"));
    assert_eq!(colored["value"], json!(false));
    assert!(colored.get("min").is_none(), "a switch has no range");

    let done = set(&view, cx, GRAIN, "colored", Raw::Bool(true)).unwrap();
    assert_eq!(done["changed"], json!(true));
    assert_eq!(field_value(&node(&view, cx, GRAIN), "colored"), json!(true));
    assert_eq!(undo_depth(&view, cx), depth + 1, "one gesture, one entry");

    // A number is not a boolean, whatever it is.
    let error = set(&view, cx, GRAIN, "colored", Raw::Number(1.0)).unwrap_err();
    assert_eq!(error.code, "invalid_value");
    assert_eq!(undo_depth(&view, cx), depth + 1);
    // Setting what it already is writes nothing.
    let same = set(&view, cx, GRAIN, "colored", Raw::Bool(true)).unwrap();
    assert_eq!(same["changed"], json!(false));
    assert_eq!(undo_depth(&view, cx), depth + 1);

    view.update(cx, |view, cx| view.execute(Command::Undo, Source::Cli, cx))
        .unwrap();
    assert_eq!(
        field_value(&node(&view, cx, GRAIN), "colored"),
        json!(false)
    );
}

#[gpui_kit::test]
fn the_seed_is_automatic_until_a_whole_uint32_is_written_and_a_vignette_has_none(
    cx: &mut TestAppContext,
) {
    let (view, cx) = mounted(cx);
    add_grain(&view, cx);
    let depth = undo_depth(&view, cx);
    let clip = node(&view, cx, GRAIN);
    let seed = clip["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["field"] == "seed")
        .unwrap()
        .clone();
    assert_eq!(seed["type"], json!("integer"));
    assert_eq!(seed["explicit"], json!(false));
    assert_eq!(seed["reseed"], json!("per-frame"));
    let automatic = seed["value"].as_u64().unwrap();
    // Nothing to restore: listed, disabled, with its reason, never a callback that does nothing.
    let auto = action(&clip, "seed:auto");
    assert_eq!(auto["enabled"], json!(false));
    assert!(auto["disabled_reason"].is_string());

    set(&view, cx, GRAIN, "seed", Raw::Number(7.0)).unwrap();
    assert_eq!(undo_depth(&view, cx), depth + 1);
    let clip = node(&view, cx, GRAIN);
    let seed = clip["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["field"] == "seed")
        .unwrap();
    assert_eq!(
        (seed["value"].clone(), seed["explicit"].clone()),
        (json!(7), json!(true))
    );
    assert_eq!(action(&clip, "seed:auto")["enabled"], json!(true));

    // The refusals are typed and leave the document alone.
    for bad in [
        Raw::Number(-1.0),
        Raw::Number(1.5),
        Raw::Number(4_294_967_296.0),
    ] {
        let error = set(&view, cx, GRAIN, "seed", bad).unwrap_err();
        assert_eq!(error.code, "invalid_value");
    }
    assert_eq!(undo_depth(&view, cx), depth + 1);
    // The largest seed is allowed.
    set(&view, cx, GRAIN, "seed", Raw::Number(4_294_967_295.0)).unwrap();

    invoke(&view, cx, GRAIN, "seed:auto").unwrap();
    let back = node(&view, cx, GRAIN);
    let seed = back["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["field"] == "seed")
        .unwrap();
    assert_eq!(seed["explicit"], json!(false));
    assert_eq!(seed["value"].as_u64().unwrap(), automatic);
    let again = invoke(&view, cx, GRAIN, "seed:auto").unwrap();
    assert_eq!(
        again["changed"],
        json!(false),
        "already automatic: no entry"
    );

    // A vignette draws no noise: no seed field, no seed action, and a typed refusal.
    invoke(&view, cx, "scene.fx", "add:vignette").unwrap();
    let vignette = node(&view, cx, "scene.fx.clip:fx-2");
    assert!(
        vignette["fields"]
            .as_array()
            .unwrap()
            .iter()
            .all(|field| field["field"] != "seed")
    );
    let error = set(&view, cx, "scene.fx.clip:fx-2", "seed", Raw::Number(3.0)).unwrap_err();
    assert_eq!(error.code, "not_applicable");
}

#[gpui_kit::test]
fn nothing_is_keyed_by_a_slider_and_a_capture_is_one_entry_at_the_frame_the_monitor_shows(
    cx: &mut TestAppContext,
) {
    let (view, cx) = mounted(cx);
    add_grain(&view, cx);
    let depth = undo_depth(&view, cx);

    // A static set never keys: the base moves, no curve appears.
    set(&view, cx, GRAIN, "amount", Raw::Number(0.9)).unwrap();
    assert_eq!(animation(&view, cx, "amount")["animated"], json!(false));
    assert_eq!(animation(&view, cx, "amount")["key_count"], json!(0));
    assert_eq!(undo_depth(&view, cx), depth + 1);

    // The capture records what the parameter shows at the frame, as one entry.
    let capture = action(&node(&view, cx, GRAIN), "key:amount");
    assert_eq!(capture["enabled"], json!(true));
    invoke(&view, cx, GRAIN, "key:amount").unwrap();
    assert_eq!(undo_depth(&view, cx), depth + 2);
    let amount = animation(&view, cx, "amount");
    assert_eq!(amount["animated"], json!(true));
    assert_eq!(amount["key_count"], json!(1));
    assert_eq!(amount["keys"][0]["frame"], json!(0));
    assert_eq!(
        amount["keys"][0]["value"],
        json!(0.9),
        "the captured value is the base"
    );

    // At the next frame the capture is a second key; the first is not touched.
    move_playhead_to_frame(&view, cx, 12);
    invoke(&view, cx, GRAIN, "key:amount").unwrap();
    assert_eq!(animation(&view, cx, "amount")["key_count"], json!(2));
    assert_eq!(
        animation(&view, cx, "amount")["keys"][1]["frame"],
        json!(12)
    );
    assert_eq!(undo_depth(&view, cx), depth + 3);

    // Moving a slider while keys prevail changes the base only; the curve stays and the core warns.
    let answer = set(&view, cx, GRAIN, "amount", Raw::Number(0.4)).unwrap();
    assert_eq!(
        animation(&view, cx, "amount")["key_count"],
        json!(2),
        "{answer}"
    );
    assert_eq!(animation(&view, cx, "amount")["base"], json!(0.4));
    assert!(
        answer["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning["code"] == json!("FX_STATIC_UNDER_KEYS")),
        "{answer}"
    );

    // Undo takes the static set; then the capture, one entry each.
    for _ in 0..2 {
        view.update(cx, |view, cx| view.execute(Command::Undo, Source::Cli, cx))
            .unwrap();
    }
    assert_eq!(animation(&view, cx, "amount")["key_count"], json!(1));
}

#[gpui_kit::test]
fn a_capture_outside_the_clip_is_listed_disabled_and_refused_with_a_typed_code(
    cx: &mut TestAppContext,
) {
    let (view, cx) = mounted(cx);
    add_grain(&view, cx);
    // Cut the clip short and move the monitor past it.
    set(&view, cx, GRAIN, "duration_frames", Raw::Number(10.0)).unwrap();
    move_playhead_to_frame(&view, cx, 20);
    let depth = undo_depth(&view, cx);
    let capture = action(&node(&view, cx, GRAIN), "key:amount");
    assert_eq!(capture["enabled"], json!(false));
    assert!(capture["disabled_reason"].is_string());
    let error = invoke(&view, cx, GRAIN, "key:amount").unwrap_err();
    assert_eq!(error.code, "frame_outside_clip");
    assert_eq!(
        undo_depth(&view, cx),
        depth,
        "a refused capture writes nothing"
    );
    // An explicit frame of the clip still works from anywhere.
    invoke(&view, cx, GRAIN, "key:amount:4").unwrap();
    assert_eq!(animation(&view, cx, "amount")["keys"][0]["frame"], json!(4));
    // A frame the clip does not hold is the core's typed refusal.
    let error = invoke(&view, cx, GRAIN, "key:amount:10").unwrap_err();
    assert_eq!(error.code, "key_out_of_clip");
}

#[gpui_kit::test]
fn keys_move_change_curve_and_go_away_each_as_one_entry_and_removing_the_animation_keeps_the_base(
    cx: &mut TestAppContext,
) {
    let (view, cx) = mounted(cx);
    add_grain(&view, cx);
    set(&view, cx, GRAIN, "size", Raw::Number(2.0)).unwrap();
    invoke(&view, cx, GRAIN, "key:size:0").unwrap();
    invoke(&view, cx, GRAIN, "key:size:10").unwrap();
    let depth = undo_depth(&view, cx);

    let clip = node(&view, cx, GRAIN);
    assert_eq!(
        action(&clip, "easing:size:10:easeOutCubic")["enabled"],
        json!(true)
    );
    invoke(&view, cx, GRAIN, "easing:size:10:easeOutCubic").unwrap();
    assert_eq!(
        animation(&view, cx, "size")["keys"][1]["easing"],
        json!("easeOutCubic")
    );
    assert_eq!(undo_depth(&view, cx), depth + 1);
    // A curve the catalog does not know is the core's typed refusal.
    let error = invoke(&view, cx, GRAIN, "easing:size:10:bounce").unwrap_err();
    assert_eq!(error.code, "unknown_keyframe_easing");

    // A key moves one frame at a time and cannot cross a neighbour (listed disabled, refused).
    invoke(&view, cx, GRAIN, "move_key:size:10:11").unwrap();
    assert_eq!(animation(&view, cx, "size")["keys"][1]["frame"], json!(11));
    let clip = node(&view, cx, GRAIN);
    assert_eq!(action(&clip, "move_key:size:0:-1")["enabled"], json!(false));
    let error = invoke(&view, cx, GRAIN, "move_key:size:11:-3").unwrap_err();
    assert!(
        ["key_out_of_clip", "keyframe_order"].contains(&error.code),
        "{}",
        error.code
    );
    let error = invoke(&view, cx, GRAIN, "move_key:size:99:5").unwrap_err();
    assert_eq!(error.code, "unknown_keyframe");
    assert_eq!(undo_depth(&view, cx), depth + 2, "refusals leave no entry");

    // A key of a component keeps its siblings: one channel of the colour of a vignette.
    invoke(&view, cx, "scene.fx", "add:vignette").unwrap();
    let vignette = "scene.fx.clip:fx-2";
    set(&view, cx, vignette, "color_g", Raw::Number(0.5)).unwrap();
    set(&view, cx, vignette, "key:color_r:3", Raw::Number(1.0)).unwrap();
    let state = clip_state(&view, cx, "fx-2");
    let color = state["animation"]["params"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["param"] == "color")
        .unwrap()
        .clone();
    assert_eq!(
        color["keys"][0]["value"],
        json!({ "r": 1.0, "g": 0.5, "b": 0.0 })
    );

    // Removing a key, then the whole animation: the base stays what it was.
    invoke(&view, cx, GRAIN, "remove_key:size:0").unwrap();
    assert_eq!(animation(&view, cx, "size")["key_count"], json!(1));
    let removed = invoke(&view, cx, GRAIN, "remove_animation:size").unwrap();
    assert_eq!(removed["changed"], json!(true));
    let size = animation(&view, cx, "size");
    assert_eq!(
        (size["animated"].clone(), size["base"].clone()),
        (json!(false), json!(2.0))
    );
    let clip = node(&view, cx, GRAIN);
    assert_eq!(
        action(&clip, "remove_animation:size")["enabled"],
        json!(false)
    );
    // A parameter nobody knows is a typed refusal.
    let error = invoke(&view, cx, GRAIN, "key:nope:1").unwrap_err();
    assert_eq!(error.code, "unknown_param");
    let error = invoke(&view, cx, GRAIN, "key:size:x").unwrap_err();
    assert_eq!(error.code, "invalid_value");
    assert_eq!(clips(&view, cx), 2);
}

#[gpui_kit::test]
fn the_buttons_of_the_panel_run_the_same_handlers_when_clicked(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    add_grain(&view, cx);
    cx.run_until_parked();
    let depth = undo_depth(&view, cx);

    // Capture with the button: the probe exists because the panel painted the row.
    let capture = center_of(&view, cx, "control:fx-key:fx-1:amount");
    cx.simulate_click(capture, Default::default());
    cx.run_until_parked();
    assert_eq!(animation(&view, cx, "amount")["key_count"], json!(1));
    assert_eq!(undo_depth(&view, cx), depth + 1);

    // The easing button cycles the curve of the key it belongs to.
    let easing = center_of(&view, cx, "control:fx-keyease:fx-1:amount:0");
    cx.simulate_click(easing, Default::default());
    cx.run_until_parked();
    assert_eq!(
        animation(&view, cx, "amount")["keys"][0]["easing"],
        json!("easeInOutCubic")
    );
    assert_eq!(undo_depth(&view, cx), depth + 2);

    // The seed steppers write an explicit seed one step from the one shown; auto restores it.
    let before = clip_state(&view, cx, "fx-1")["animation"]["seed"]["base"]
        .as_u64()
        .unwrap();
    let up = center_of(&view, cx, "control:fx-seed-step:fx-1:1");
    cx.simulate_click(up, Default::default());
    cx.run_until_parked();
    let seed = clip_state(&view, cx, "fx-1")["animation"]["seed"].clone();
    let notice = view.read_with(cx, |view, _| view.activity.as_ref().map(|a| a.text.clone()));
    assert_eq!(seed["explicit"], json!(before + 1), "{notice:?}");
    let auto = center_of(&view, cx, "control:fx-seed-auto:fx-1");
    cx.simulate_click(auto, Default::default());
    cx.run_until_parked();
    assert_eq!(
        clip_state(&view, cx, "fx-1")["animation"]["seed"]["from_id"],
        json!(true)
    );

    // The switch is a real Kit Switch: its probe is on the screen and a click flips the boolean.
    let colored = center_of(&view, cx, "control:fx:fx-1:colored");
    cx.simulate_click(colored, Default::default());
    cx.run_until_parked();
    assert_eq!(field_value(&node(&view, cx, GRAIN), "colored"), json!(true));

    // Removing the key with its button leaves the parameter static again.
    let remove = center_of(&view, cx, "control:fx-keyrm:fx-1:amount:0");
    cx.simulate_click(remove, Default::default());
    cx.run_until_parked();
    assert_eq!(animation(&view, cx, "amount")["animated"], json!(false));
}
