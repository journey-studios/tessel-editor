//! F058 (BL082.30, P3): the typed editors of the key controls of the "Efeitos" section — the frame
//! and the value of every key, the seed, the shift and scale of a curve, the navigation between
//! keys and the base value under keys — driven the way a person drives them: by typing into the
//! real Kit inputs and confirming with Enter or leaving the field, and by clicking the real
//! buttons the panel paints, and by the semantic nodes that share their handlers.
//!
//! The rules (the keys, their order, the sampling, the seek arithmetic, the refusals) are the
//! core's and are tested in `crates/core/tests/scene_fx_keys.rs`, `scene_fx_seek.rs` and
//! `scene_fx_key_bounds.rs`; these tests prove the window reaches them: ONE commit and ONE undo
//! entry per typed value (Enter then Blur never writes twice), a text that is not a number is a
//! typed error that writes nothing (never a 0), an edit built on a revision that moved is refused,
//! the siblings of a point or a colour are the key's own, the monitor goes to the key the core
//! names (with the sequence at NTSC and the clip not at frame 0), and an animated parameter's
//! slider is its disabled base. Not proved here: pixels in the monitor, a real pointer or the
//! real GPU.
use super::scene_fx::FxKey;
use super::scene_fx_editors::{EditorField, EditorKey};
use super::scene_fx_keys_tests::tall;
use super::scene_fx_tests::{
    center_of, clip_state, field_value, invoke, mounted, node, set, undo_depth,
};
use super::*;
// The Kit's test macro expands to `#[test]`; the glob import above would resolve it to the Kit macro again.
use core::prelude::v1::test;
use tessel_editor_core::scene_edit::FxOp;
use tessel_editor_core::scene_project::SceneCommand;
use tessel_editor_core::time::Rate;

const GRAIN: &str = "scene.fx.clip:fx-1";

fn key_of(field: EditorField) -> EditorKey {
    EditorKey {
        scene: "street".into(),
        clip: "fx-1".into(),
        field,
    }
}

fn value_key(control: &str, frame: i64) -> EditorKey {
    key_of(EditorField::KeyValue {
        control: control.into(),
        frame,
    })
}

fn frame_key(param: &str, frame: i64) -> EditorKey {
    key_of(EditorField::KeyFrame {
        param: param.into(),
        frame,
    })
}

fn repaint(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn keys(view: &Entity<Workstation>, cx: &mut VisualTestContext, param: &str) -> Vec<Value> {
    clip_state(view, cx, "fx-1")["animation"]["params"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["param"] == param)
        .unwrap_or_else(|| panic!("no animation for `{param}`"))["keys"]
        .as_array()
        .unwrap()
        .clone()
}

fn frames(view: &Entity<Workstation>, cx: &mut VisualTestContext, param: &str) -> Vec<i64> {
    keys(view, cx, param)
        .iter()
        .map(|key| key["frame"].as_i64().unwrap())
        .collect()
}

fn text_of(view: &Entity<Workstation>, cx: &mut VisualTestContext, key: &EditorKey) -> String {
    view.read_with(cx, |view, cx| view.fx_editor_text(key, cx))
        .unwrap_or_else(|| panic!("no editor {key:?}"))
}

fn notice(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> String {
    view.read_with(cx, |view, _| {
        view.activity
            .as_ref()
            .map(|activity| activity.text.clone())
            .unwrap_or_default()
    })
}

/// Types `text` into the editor like a person: the field holds it, nothing is committed yet.
fn type_into(view: &Entity<Workstation>, cx: &mut VisualTestContext, key: &EditorKey, text: &str) {
    let input = view
        .read_with(cx, |view, _| view.fx_editor_input(key))
        .unwrap_or_else(|| panic!("the panel painted no editor {key:?}"));
    view.update_in(cx, |_, window, cx| {
        input.update(cx, |input, cx| {
            input.set_value(text.to_string(), window, cx)
        });
    });
    repaint(cx);
}

fn enter(view: &Entity<Workstation>, cx: &mut VisualTestContext, key: &EditorKey) {
    let input = view
        .read_with(cx, |view, _| view.fx_editor_input(key))
        .expect("an editor");
    input.update(cx, |_, cx| {
        cx.emit(InputEvent::PressEnter {
            secondary: false,
            shift: false,
        });
    });
    repaint(cx);
}

fn leave(view: &Entity<Workstation>, cx: &mut VisualTestContext, key: &EditorKey) {
    let input = view
        .read_with(cx, |view, _| view.fx_editor_input(key))
        .expect("an editor");
    input.update(cx, |_, cx| cx.emit(InputEvent::Blur));
    repaint(cx);
}

fn grain_with_a_key(cx: &mut TestAppContext) -> (Entity<Workstation>, &mut VisualTestContext) {
    let (view, cx) = mounted(cx);
    tall(cx);
    invoke(&view, cx, "scene.fx", "add:filmGrain").unwrap();
    invoke(&view, cx, GRAIN, "key:amount").unwrap();
    repaint(cx);
    (view, cx)
}

#[gpui_kit::test]
fn a_typed_value_commits_once_on_enter_and_the_blur_that_follows_writes_nothing(
    cx: &mut TestAppContext,
) {
    let (view, cx) = grain_with_a_key(cx);
    let depth = undo_depth(&view, cx);
    let value = value_key("amount", 0);
    let shown = text_of(&view, cx, &value);
    assert_eq!(shown, "0.25", "the editor shows the key's value");

    type_into(&view, cx, &value, "0.8");
    assert_eq!(
        keys(&view, cx, "amount")[0]["value"],
        json!(0.25),
        "typing writes nothing"
    );
    assert_eq!(undo_depth(&view, cx), depth);
    enter(&view, cx, &value);
    assert_eq!(keys(&view, cx, "amount")[0]["value"], json!(0.8));
    assert_eq!(undo_depth(&view, cx), depth + 1, "one commit, one entry");
    // The Blur that follows an Enter, and a second Enter, find nothing to commit.
    leave(&view, cx, &value);
    enter(&view, cx, &value);
    assert_eq!(undo_depth(&view, cx), depth + 1);
    assert_eq!(text_of(&view, cx, &value), "0.8");

    // Leaving the field commits too (once), and an untouched field commits nothing.
    type_into(&view, cx, &value, "0.4");
    leave(&view, cx, &value);
    assert_eq!(keys(&view, cx, "amount")[0]["value"], json!(0.4));
    assert_eq!(undo_depth(&view, cx), depth + 2);
    leave(&view, cx, &value);
    assert_eq!(undo_depth(&view, cx), depth + 2);

    // Undo takes the commit as ONE entry and the editor follows the model.
    view.update(cx, |view, cx| view.execute(Command::Undo, Source::Cli, cx))
        .unwrap();
    repaint(cx);
    assert_eq!(keys(&view, cx, "amount")[0]["value"], json!(0.8));
    assert_eq!(text_of(&view, cx, &value), "0.8");
}

#[gpui_kit::test]
fn a_typed_frame_moves_the_key_and_a_busy_frame_is_refused_with_nothing_written(
    cx: &mut TestAppContext,
) {
    let (view, cx) = grain_with_a_key(cx);
    // A second key to collide with.
    set(&view, cx, GRAIN, "key:amount:12", Raw::Number(0.9)).unwrap();
    repaint(cx);
    let depth = undo_depth(&view, cx);
    assert_eq!(frames(&view, cx, "amount"), [0, 12]);

    let first = frame_key("amount", 0);
    assert_eq!(text_of(&view, cx, &first), "0");
    type_into(&view, cx, &first, "7");
    enter(&view, cx, &first);
    assert_eq!(frames(&view, cx, "amount"), [7, 12], "an arbitrary frame");
    assert_eq!(undo_depth(&view, cx), depth + 1);
    // The editor of the old frame is gone, the one of the new frame shows it.
    assert_eq!(text_of(&view, cx, &frame_key("amount", 7)), "7");
    assert!(view.read_with(cx, |view, _| {
        view.fx_editor_input(&frame_key("amount", 0)).is_none()
    }));

    // A frame another key holds, or past the clip, or that is not a whole number: typed refusal.
    for text in ["12", "100000", "-1", "3.5", "abc", ""] {
        let key = frame_key("amount", 7);
        type_into(&view, cx, &key, text);
        enter(&view, cx, &key);
        assert_eq!(
            frames(&view, cx, "amount"),
            [7, 12],
            "`{text}` writes nothing"
        );
        assert_eq!(undo_depth(&view, cx), depth + 1, "`{text}`");
        assert!(
            view.read_with(cx, |view, _| view
                .activity
                .as_ref()
                .is_some_and(|a| a.error)),
            "`{text}`: {}",
            notice(&view, cx)
        );
        assert_eq!(
            text_of(&view, cx, &key),
            "7",
            "the field shows the model again"
        );
    }
}

#[gpui_kit::test]
fn a_text_that_is_not_a_number_is_a_typed_error_and_never_a_zero(cx: &mut TestAppContext) {
    let (view, cx) = grain_with_a_key(cx);
    let depth = undo_depth(&view, cx);
    let value = value_key("amount", 0);
    for text in ["abc", "", "  ", "1e999", "NaN", "inf", "0,5"] {
        type_into(&view, cx, &value, text);
        enter(&view, cx, &value);
        assert_eq!(
            keys(&view, cx, "amount")[0]["value"],
            json!(0.25),
            "`{text}` wrote nothing"
        );
        assert_eq!(undo_depth(&view, cx), depth, "`{text}`");
        assert!(
            notice(&view, cx).starts_with("invalid_value"),
            "`{text}`: {}",
            notice(&view, cx)
        );
        assert_eq!(
            text_of(&view, cx, &value),
            "0.25",
            "`{text}`: the model's value is back"
        );
    }
    // A value outside the range of the parameter is the core's typed refusal.
    type_into(&view, cx, &value, "7");
    enter(&view, cx, &value);
    assert!(
        notice(&view, cx).starts_with("param_out_of_range"),
        "{}",
        notice(&view, cx)
    );
    assert_eq!(keys(&view, cx, "amount")[0]["value"], json!(0.25));
    assert_eq!(undo_depth(&view, cx), depth);
}

#[gpui_kit::test]
fn the_seed_is_typed_as_a_uint32_and_a_bad_one_writes_nothing(cx: &mut TestAppContext) {
    let (view, cx) = grain_with_a_key(cx);
    let depth = undo_depth(&view, cx);
    let seed = key_of(EditorField::Seed);
    let automatic = text_of(&view, cx, &seed);
    assert!(
        automatic.parse::<u32>().is_ok(),
        "the automatic seed is shown"
    );
    type_into(&view, cx, &seed, "123456789");
    enter(&view, cx, &seed);
    let state = clip_state(&view, cx, "fx-1")["animation"]["seed"].clone();
    assert_eq!(state["explicit"], json!(123456789));
    assert_eq!(undo_depth(&view, cx), depth + 1);
    leave(&view, cx, &seed);
    assert_eq!(
        undo_depth(&view, cx),
        depth + 1,
        "Blur after Enter: nothing"
    );

    for text in ["4294967296", "-1", "1.5", "abc", ""] {
        type_into(&view, cx, &seed, text);
        enter(&view, cx, &seed);
        assert_eq!(undo_depth(&view, cx), depth + 1, "`{text}`");
        assert!(
            notice(&view, cx).starts_with("invalid_value"),
            "`{text}`: {}",
            notice(&view, cx)
        );
        assert_eq!(text_of(&view, cx, &seed), "123456789", "`{text}`");
    }
    type_into(&view, cx, &seed, "4294967295");
    enter(&view, cx, &seed);
    assert_eq!(
        clip_state(&view, cx, "fx-1")["animation"]["seed"]["explicit"],
        json!(4294967295u32),
        "the largest uint32 is a seed"
    );
}

#[gpui_kit::test]
fn an_edit_built_on_a_revision_that_moved_is_refused_and_the_draft_never_overwrites(
    cx: &mut TestAppContext,
) {
    let (view, cx) = grain_with_a_key(cx);
    let value = value_key("amount", 0);
    // The person types, and before confirming the project moves (a script keys another frame).
    type_into(&view, cx, &value, "0.9");
    set(&view, cx, GRAIN, "key:amount:20", Raw::Number(0.5)).unwrap();
    repaint(cx);
    assert_eq!(
        text_of(&view, cx, &value),
        "0.9",
        "the draft is the person's, the model's repaint does not eat it"
    );
    let depth = undo_depth(&view, cx);
    enter(&view, cx, &value);
    assert!(
        notice(&view, cx).starts_with("stale_revision"),
        "{}",
        notice(&view, cx)
    );
    assert_eq!(
        keys(&view, cx, "amount")[0]["value"],
        json!(0.25),
        "nothing written"
    );
    assert_eq!(undo_depth(&view, cx), depth);
    assert_eq!(
        text_of(&view, cx, &value),
        "0.25",
        "the model's value is back"
    );
    // Typed again on the fresh view, it commits.
    type_into(&view, cx, &value, "0.9");
    enter(&view, cx, &value);
    assert_eq!(keys(&view, cx, "amount")[0]["value"], json!(0.9));
    assert_eq!(undo_depth(&view, cx), depth + 1);
}

#[gpui_kit::test]
fn a_key_that_went_away_under_the_editor_is_not_written_back(cx: &mut TestAppContext) {
    let (view, cx) = grain_with_a_key(cx);
    let value = value_key("amount", 0);
    type_into(&view, cx, &value, "0.9");
    invoke(&view, cx, GRAIN, "remove_key:amount:0").unwrap();
    repaint(cx);
    let depth = undo_depth(&view, cx);
    // The panel dropped the editor of a key the document no longer holds.
    assert!(
        view.read_with(cx, |view, _| view.fx_editor_input(&value).is_none()),
        "no editor for a missing key"
    );
    // Even a late commit that reached the handler is refused.
    let refused = view.update(cx, |view, cx| {
        view.fx_editor_apply(&value, "0.9", view.state.project.revision, cx)
    });
    assert_eq!(refused.unwrap_err().code, "unknown_keyframe");
    assert_eq!(undo_depth(&view, cx), depth);
    assert!(frames(&view, cx, "amount").is_empty());
}

#[gpui_kit::test]
fn typing_one_component_keeps_the_siblings_of_the_same_key(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    tall(cx);
    invoke(&view, cx, "scene.fx", "add:vignette").unwrap();
    // A point and a colour with every component distinct, keyed at frame 0.
    set(&view, cx, GRAIN, "center_x", Raw::Number(0.2)).unwrap();
    set(&view, cx, GRAIN, "center_y", Raw::Number(0.7)).unwrap();
    set(&view, cx, GRAIN, "color_r", Raw::Number(0.1)).unwrap();
    set(&view, cx, GRAIN, "color_g", Raw::Number(0.5)).unwrap();
    set(&view, cx, GRAIN, "color_b", Raw::Number(0.9)).unwrap();
    invoke(&view, cx, GRAIN, "key:center").unwrap();
    invoke(&view, cx, GRAIN, "key:color").unwrap();
    repaint(cx);
    let depth = undo_depth(&view, cx);

    let center_x = value_key("center_x", 0);
    assert_eq!(text_of(&view, cx, &center_x), "0.2");
    type_into(&view, cx, &center_x, "0.35");
    enter(&view, cx, &center_x);
    let center = keys(&view, cx, "center")[0]["value"].clone();
    assert_eq!(center, json!({"x": 0.35, "y": 0.7}), "y is the key's own");
    let color_g = value_key("color_g", 0);
    type_into(&view, cx, &color_g, "0.6");
    enter(&view, cx, &color_g);
    assert_eq!(
        keys(&view, cx, "color")[0]["value"],
        json!({"r": 0.1, "g": 0.6, "b": 0.9}),
        "r and b are the key's own"
    );
    assert_eq!(
        undo_depth(&view, cx),
        depth + 2,
        "one entry per typed value"
    );
    // Another key of the same curve is a different frame with its own values.
    set(&view, cx, GRAIN, "key:center_x:10", Raw::Number(0.8)).unwrap();
    repaint(cx);
    let keys_now = keys(&view, cx, "center");
    assert_eq!(keys_now.len(), 2);
    assert_eq!(keys_now[0]["value"], json!({"x": 0.35, "y": 0.7}));
    assert_eq!(text_of(&view, cx, &value_key("center_x", 10)), "0.8");
}

#[gpui_kit::test]
fn shift_and_scale_apply_from_their_drafts_with_one_entry_and_typed_errors(
    cx: &mut TestAppContext,
) {
    let (view, cx) = grain_with_a_key(cx);
    set(&view, cx, GRAIN, "key:amount:10", Raw::Number(0.9)).unwrap();
    repaint(cx);
    let depth = undo_depth(&view, cx);
    let shift = key_of(EditorField::Shift {
        param: "amount".into(),
    });
    assert_eq!(
        text_of(&view, cx, &shift),
        "0",
        "the draft starts at no shift"
    );

    // Typing the draft writes nothing; its button applies it, once.
    type_into(&view, cx, &shift, "5");
    assert_eq!(frames(&view, cx, "amount"), [0, 10]);
    let apply = center_of(&view, cx, "control:fx-shift:fx-1:amount");
    cx.simulate_click(apply, Default::default());
    repaint(cx);
    assert_eq!(frames(&view, cx, "amount"), [5, 15]);
    assert_eq!(undo_depth(&view, cx), depth + 1);
    assert_eq!(
        text_of(&view, cx, &shift),
        "0",
        "the draft is back to its default"
    );
    // Leaving a draft applies nothing; Enter applies it.
    type_into(&view, cx, &shift, "-5");
    leave(&view, cx, &shift);
    assert_eq!(frames(&view, cx, "amount"), [5, 15]);
    enter(&view, cx, &shift);
    assert_eq!(frames(&view, cx, "amount"), [0, 10]);
    assert_eq!(undo_depth(&view, cx), depth + 2);
    // A shift that pushes a key out of the clip is refused whole.
    type_into(&view, cx, &shift, "100000");
    enter(&view, cx, &shift);
    assert_eq!(frames(&view, cx, "amount"), [0, 10]);
    assert_eq!(undo_depth(&view, cx), depth + 2);
    assert!(view.read_with(cx, |view, _| {
        view.activity.as_ref().is_some_and(|a| a.error)
    }));
    // Not a whole number: typed, nothing written.
    type_into(&view, cx, &shift, "x");
    enter(&view, cx, &shift);
    assert!(
        notice(&view, cx).starts_with("invalid_value"),
        "{}",
        notice(&view, cx)
    );
    assert_eq!(undo_depth(&view, cx), depth + 2);

    // Scale: anchor 0, 2/1 doubles the distance; the first key stays.
    let anchor = key_of(EditorField::ScaleAnchor {
        param: "amount".into(),
    });
    assert_eq!(
        text_of(&view, cx, &anchor),
        "0",
        "the anchor starts at the first key"
    );
    let num = key_of(EditorField::ScaleNum {
        param: "amount".into(),
    });
    type_into(&view, cx, &num, "2");
    let apply = center_of(&view, cx, "control:fx-scale:fx-1:amount");
    cx.simulate_click(apply, Default::default());
    repaint(cx);
    assert_eq!(
        frames(&view, cx, "amount"),
        [0, 20],
        "{}",
        notice(&view, cx)
    );
    assert_eq!(undo_depth(&view, cx), depth + 3);
    assert_eq!(text_of(&view, cx, &num), "1");
    // Zero or a word as a ratio: typed, nothing written.
    type_into(&view, cx, &num, "0");
    let apply = center_of(&view, cx, "control:fx-scale:fx-1:amount");
    cx.simulate_click(apply, Default::default());
    repaint(cx);
    assert_eq!(
        frames(&view, cx, "amount"),
        [0, 20],
        "{}",
        notice(&view, cx)
    );
    assert_eq!(undo_depth(&view, cx), depth + 3);
    assert!(view.read_with(cx, |view, _| {
        view.activity.as_ref().is_some_and(|a| a.error)
    }));
    // One undo takes the whole scale.
    view.update(cx, |view, cx| view.execute(Command::Undo, Source::Cli, cx))
        .unwrap();
    assert_eq!(frames(&view, cx, "amount"), [0, 10]);
}

#[gpui_kit::test]
fn an_animated_parameters_slider_is_its_disabled_base_and_never_keys_or_writes(
    cx: &mut TestAppContext,
) {
    let (view, cx) = grain_with_a_key(cx);
    let depth = undo_depth(&view, cx);
    let amount = node(&view, cx, GRAIN);
    let base = amount["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["field"] == "amount")
        .unwrap()
        .clone();
    assert_eq!(base["enabled"], json!(false));
    assert_eq!(base["animated"], json!(true));
    assert!(base["label"].as_str().unwrap().ends_with("(base)"));
    // The other parameters of the same clip are still plain sliders.
    let size = amount["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["field"] == "size")
        .unwrap()
        .clone();
    assert_eq!(size["enabled"], json!(true));
    assert!(size.get("animated").is_none());

    // A key press on the disabled slider is not a step.
    let key = FxKey {
        scene: "street".into(),
        clip: "fx-1".into(),
        control: "amount".into(),
    };
    let handled = view.update(cx, |view, cx| {
        view.fx_key_down(&key, "right", false, false, cx)
    });
    assert!(!handled, "the base of an animated parameter takes no key");
    let other = FxKey {
        control: "size".into(),
        ..key
    };
    let handled = view.update(cx, |view, cx| {
        view.fx_key_down(&other, "right", false, false, cx)
    });
    assert!(handled, "a static parameter still does");
    view.update(cx, |view, cx| view.fx_released(&other, cx));
    assert_eq!(
        undo_depth(&view, cx),
        depth + 1,
        "only the static one wrote"
    );
    assert_eq!(
        keys(&view, cx, "amount").len(),
        1,
        "nothing was keyed by moving a slider"
    );

    // Resetting the base is an explicit write of the base and keeps the curve.
    set(&view, cx, GRAIN, "amount", Raw::Number(0.5)).unwrap_err();
    invoke(&view, cx, GRAIN, "reset:amount").ok();
    assert_eq!(
        keys(&view, cx, "amount").len(),
        1,
        "the keys survive a reset"
    );
}

#[gpui_kit::test]
fn the_navigation_buttons_move_the_monitor_to_the_keys_the_core_names(cx: &mut TestAppContext) {
    let (view, cx) = grain_with_a_key(cx);
    set(&view, cx, GRAIN, "key:amount:10", Raw::Number(0.9)).unwrap();
    set(&view, cx, GRAIN, "key:amount:20", Raw::Number(0.5)).unwrap();
    repaint(cx);
    let depth = undo_depth(&view, cx);
    let here = |view: &Entity<Workstation>, cx: &mut VisualTestContext| {
        node(view, cx, "scene.fx")["state"]["playhead_frame"].clone()
    };
    let go = |view: &Entity<Workstation>, cx: &mut VisualTestContext, to: &str| {
        let at = center_of(view, cx, &format!("control:fx-go:fx-1:amount:{to}"));
        cx.simulate_click(at, Default::default());
        repaint(cx);
    };
    go(&view, cx, "last");
    assert_eq!(here(&view, cx), json!(20));
    go(&view, cx, "previous");
    assert_eq!(here(&view, cx), json!(10));
    go(&view, cx, "next");
    assert_eq!(here(&view, cx), json!(20));
    go(&view, cx, "first");
    assert_eq!(here(&view, cx), json!(0));
    // The go-to-this-key button of a row.
    let row = center_of(&view, cx, "control:fx-keygo:fx-1:amount:10");
    cx.simulate_click(row, Default::default());
    repaint(cx);
    assert_eq!(here(&view, cx), json!(10));
    assert_eq!(
        undo_depth(&view, cx),
        depth,
        "going to a key writes nothing"
    );

    // At the last key `next` is listed disabled with the seek's own code, and refused when invoked.
    go(&view, cx, "last");
    let clip = node(&view, cx, GRAIN);
    let next = clip["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|action| action["action"] == "go:amount:next")
        .unwrap()
        .clone();
    assert_eq!(next["enabled"], json!(false));
    assert!(next["disabled_reason"].is_string());
    let refused = invoke(&view, cx, GRAIN, "go:amount:next").unwrap_err();
    assert_eq!(refused.code, "no_next_key");
    assert_eq!(here(&view, cx), json!(20), "a refusal moves nothing");
    // The node reaches the same handler, by name and by frame.
    invoke(&view, cx, GRAIN, "go:amount:10").unwrap();
    assert_eq!(here(&view, cx), json!(10));
    assert_eq!(
        invoke(&view, cx, GRAIN, "go:amount:11").unwrap_err().code,
        "unknown_keyframe"
    );
}

#[gpui_kit::test]
fn going_to_a_key_is_exact_at_ntsc_with_the_clip_not_at_frame_zero(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    tall(cx);
    // A sequence at 30000/1001 over the 24 fps scene, and a grain that starts later than frame 0.
    view.update(cx, |view, _| {
        view.state.timeline.rate = Rate::new(30000, 1001).unwrap();
        view.state.timeline.set_playhead(15);
    });
    invoke(&view, cx, "scene.fx", "add:filmGrain").unwrap();
    let start = clip_state(&view, cx, "fx-1")["start_frame"]
        .as_i64()
        .unwrap();
    assert!(start > 0, "the clip starts at {start}, not at 0");
    for (frame, value) in [(0, 0.1), (4, 0.5), (9, 0.9)] {
        set(
            &view,
            cx,
            GRAIN,
            &format!("key:amount:{frame}"),
            Raw::Number(value),
        )
        .unwrap();
    }
    repaint(cx);
    let depth = undo_depth(&view, cx);
    let shown = |view: &Entity<Workstation>, cx: &mut VisualTestContext| {
        node(view, cx, "scene.fx")["state"]["playhead_frame"]
            .as_i64()
            .unwrap()
    };
    for (to, local) in [("first", 0), ("next", 4), ("next", 9), ("previous", 4)] {
        invoke(&view, cx, GRAIN, &format!("go:amount:{to}")).unwrap();
        assert_eq!(
            shown(&view, cx),
            start + local,
            "{to}: the monitor shows the key's frame"
        );
        let at = view.read_with(cx, |view, _| view.state.timeline.playhead);
        let exact = view.read_with(cx, |view, _| {
            view.state.scene_frame_at_playhead("street").unwrap()
        });
        assert_eq!(exact, start + local, "{to}: sequence frame {at}");
    }
    assert_eq!(undo_depth(&view, cx), depth, "seeking is not an edit");
    // The inspector's own "agora" value is the curve's value at that frame.
    let animation = clip_state(&view, cx, "fx-1")["animation"].clone();
    assert_eq!(animation["local_frame"], json!(4));
    assert_eq!(
        field_value(&node(&view, cx, GRAIN), "key:amount:4"),
        json!(0.5)
    );
}

#[gpui_kit::test]
fn the_editors_are_listed_as_fields_of_the_clip_node_and_run_the_same_handlers(
    cx: &mut TestAppContext,
) {
    let (view, cx) = grain_with_a_key(cx);
    let clip = node(&view, cx, GRAIN);
    let names: Vec<String> = clip["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|field| field["field"].as_str().unwrap().to_string())
        .collect();
    for expected in [
        "seed",
        "key_frame:amount:0",
        "key:amount:0",
        "shift_keys:amount",
    ] {
        assert!(
            names.iter().any(|name| name == expected),
            "{expected} in {names:?}"
        );
    }
    let depth = undo_depth(&view, cx);
    // The node's `set` is what the editor's commit runs.
    set(&view, cx, GRAIN, "key_frame:amount:0", Raw::Number(6.0)).unwrap();
    assert_eq!(frames(&view, cx, "amount"), [6]);
    set(&view, cx, GRAIN, "shift_keys:amount", Raw::Number(2.0)).unwrap();
    assert_eq!(frames(&view, cx, "amount"), [8]);
    invoke(&view, cx, GRAIN, "scale_keys:amount:0:3:2").unwrap();
    assert_eq!(
        frames(&view, cx, "amount"),
        [12],
        "8 is 8 away from 0, times 3/2"
    );
    assert_eq!(undo_depth(&view, cx), depth + 3);
    assert_eq!(
        set(&view, cx, GRAIN, "key:amount:12", Raw::Text("abc".into()))
            .unwrap_err()
            .code,
        "invalid_value"
    );
    // The same refusal from the library, built on a revision that moved.
    let stale = view.update(cx, |view, cx| {
        view.execute(
            Command::Scene(SceneCommand::FxEdit {
                scene: "street".into(),
                ops: vec![FxOp::ClearKeys {
                    clip: "fx-1".into(),
                    param: Some("amount".into()),
                }],
                dry_run: false,
                base_revision: Some(view.state.project.revision.saturating_sub(1)),
            }),
            Source::Cli,
            cx,
        )
    });
    assert_eq!(stale.unwrap_err().code, "stale_revision");
    assert_eq!(frames(&view, cx, "amount"), [12], "nothing was cleared");
}
