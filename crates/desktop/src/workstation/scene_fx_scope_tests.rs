//! F062: the scope of an effect clip in the window — the selector on the clip, the choice and the
//! actions on its node, the shots on the section and the action that adds on the shot the monitor
//! shows — driven through the semantic nodes, through the Kit select and by real GPUI clicks over the
//! buttons that paint them.
//!
//! The rules (what a scope is, which shots hold which clips, the typed refusals, the composition in
//! stages) are the core's and are tested in `crates/core/tests/scene_fx_shot_scope.rs`; the pixels are
//! in `crates/render/tests/scene_fx_shot_scope.rs`. These tests prove the window reaches the core:
//! one op and one undo entry per gesture, the typed refusal on screen with nothing written, the
//! disabled controls with their reason, and a node surface that says what the panel shows. The scene
//! is the corpus `valid-minimal-continuous`, whose two shots are `[0, 48)` and `[48, 96)`.
//!
//! Not proved here: pixels in the monitor and a physical mouse over the Kit select (the pick is
//! the select's own `Confirm` event, which the Kit emits on a click).
use gpui_kit::component::select::SelectEvent;

use super::scene_fx_tests::{
    attach_yard, center_of, choose, clips, invoke, mounted, node, set, total_depth, undo_depth,
};
use super::*;
// The Kit's test macro expands to `#[test]`; the glob import above would resolve it to the Kit macro again.
use core::prelude::v1::test;
use tessel_editor_core::directed_scene::fx::FxScope;

const SECTION: &str = "scene.fx";
const FIRST: &str = "scene.fx.clip:fx-1";

fn field(node: &Value, name: &str) -> Value {
    node["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["field"] == name)
        .unwrap_or_else(|| panic!("no field `{name}` on {}", node["id"]))
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

/// Walks the NLE playhead until the monitor shows a frame of `shot`; that frame of the scene.
fn playhead_in_shot(view: &Entity<Workstation>, cx: &mut VisualTestContext, shot: &str) -> i64 {
    for candidate in 0..2000 {
        view.update(cx, |view, _| view.state.timeline.set_playhead(candidate));
        let state = node(view, cx, SECTION)["state"].clone();
        if state["shot_here"] == json!(shot) {
            return state["playhead_frame"].as_i64().unwrap();
        }
    }
    panic!("no playhead position shows a frame of `{shot}`");
}

/// Moves the NLE playhead past the end of the scene's edit, where the monitor shows a frame that no
/// shot holds (the time still converts to a frame number; the shots end where the edit ends).
fn playhead_outside_the_edit(view: &Entity<Workstation>, cx: &mut VisualTestContext) {
    for candidate in (0..20_000).step_by(25) {
        view.update(cx, |view, _| view.state.timeline.set_playhead(candidate));
        let state = node(view, cx, SECTION)["state"].clone();
        if state["shot_here"].is_null() {
            assert!(
                state["playhead_frame"].as_i64().unwrap()
                    >= state["edit_duration_frames"].as_i64().unwrap(),
                "a frame in the edit is in a shot: {state}"
            );
            // A paint follows the move: the buttons and the probes are those of this frame.
            view.update(cx, |_, cx| cx.notify());
            cx.run_until_parked();
            return;
        }
    }
    panic!("the timeline has no position past the edit of the scene");
}

fn scope_of(view: &Entity<Workstation>, cx: &mut VisualTestContext, clip: &str) -> FxScope {
    scope_in(view, cx, "street", clip)
}

fn scope_in(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    scene: &str,
    clip: &str,
) -> FxScope {
    view.read_with(cx, |view, _| {
        view.state
            .scenes
            .get(scene)
            .unwrap()
            .scene
            .bundle()
            .fx
            .tracks
            .iter()
            .flat_map(|track| &track.clips)
            .find(|candidate| candidate.id == clip)
            .unwrap_or_else(|| panic!("no clip `{clip}`"))
            .scope
            .clone()
    })
}

fn clip_span(view: &Entity<Workstation>, cx: &mut VisualTestContext, clip: &str) -> (i64, i64) {
    view.read_with(cx, |view, _| {
        let attached = view.state.scenes.get("street").unwrap();
        let clip = attached
            .scene
            .bundle()
            .fx
            .tracks
            .iter()
            .flat_map(|track| &track.clips)
            .find(|candidate| candidate.id == clip)
            .unwrap_or_else(|| panic!("no clip `{clip}`"))
            .clone();
        (clip.start_frame, clip.end_frame_exclusive())
    })
}

fn notice_code(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Option<String> {
    view.read_with(cx, |view, _| {
        view.scene_ui
            .notice
            .as_ref()
            .map(|notice| notice.code.clone())
    })
}

fn revision(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> u64 {
    view.read_with(cx, |view, _| view.state.project.revision)
}

#[gpui_kit::test]
fn the_nodes_list_the_scope_choice_the_shots_and_what_fits_where(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    let section = node(&view, cx, SECTION);
    // What the build draws, the shots of the edit and the one the monitor shows.
    assert_eq!(section["state"]["scope"], json!(["output", "shot"]));
    let shots = section["state"]["shots"].as_array().unwrap();
    assert_eq!(shots.len(), 2);
    assert_eq!(
        (
            &shots[0]["id"],
            &shots[0]["start_frame"],
            &shots[0]["end_frame_exclusive"]
        ),
        (&json!("shot-1"), &json!(0), &json!(48))
    );
    assert_eq!(
        (
            &shots[1]["id"],
            &shots[1]["start_frame"],
            &shots[1]["end_frame_exclusive"]
        ),
        (&json!("shot-2"), &json!(48), &json!(96))
    );
    assert_eq!(section["state"]["shot_here"], json!("shot-1"));
    for effect in ["vignette", "fade", "filmGrain"] {
        let add = action(&section, &format!("add_in_shot:{effect}"));
        assert_eq!(add["enabled"], json!(true), "{effect}");
        assert_eq!(add["disabled_reason"], Value::Null, "{effect}");
    }

    // `add:vignette` lasts the effect's 96 frames, across both shots: the choice says it fits the
    // output only, and why not each shot, with the code the core would refuse with.
    invoke(&view, cx, SECTION, "add:vignette").unwrap();
    let clip = node(&view, cx, FIRST);
    let scope = field(&clip, "scope");
    assert_eq!(scope["type"], json!("choice"));
    assert_eq!(scope["value"], json!("output"));
    assert_eq!(scope["default"], json!("output"));
    assert_eq!(scope["at_default"], json!(true));
    assert_eq!(scope["enabled"], json!(true));
    let options = scope["options"].as_array().unwrap();
    assert_eq!(
        options
            .iter()
            .map(|option| option["value"].clone())
            .collect::<Vec<_>>(),
        [json!("output"), json!("shot:shot-1"), json!("shot:shot-2")]
    );
    assert_eq!(options[0]["applicable"], json!(true));
    for option in &options[1..] {
        assert_eq!(option["applicable"], json!(false), "{option}");
        assert_eq!(option["reason"], json!("FX_SCOPE_MISMATCH"), "{option}");
    }
    // Every option is an action too: the scope it already holds and a shot that does not hold the
    // clip are disabled, each with its reason, never a callback that does nothing.
    for name in ["output", "shot:shot-1", "shot:shot-2"] {
        let action = action(&clip, &format!("set_scope:{name}"));
        assert_eq!(action["enabled"], json!(false), "{name}");
        assert!(action["disabled_reason"].is_string(), "{name}");
    }
    assert_eq!(clip["state"]["scope"], json!("output"));
    assert_eq!(clip["state"]["scope_label"], json!("Saída"));
    assert_eq!(clip["state"]["shot"], Value::Null);
}

#[gpui_kit::test]
fn add_in_shot_lays_the_clip_inside_the_shot_the_monitor_shows_as_one_entry(
    cx: &mut TestAppContext,
) {
    let (view, cx) = mounted(cx);
    let depth = undo_depth(&view, cx);

    let first = invoke(&view, cx, SECTION, "add_in_shot:vignette").unwrap();
    assert_eq!(first["changed"], json!(true));
    assert_eq!(clips(&view, cx), 1);
    assert_eq!(undo_depth(&view, cx), depth + 1, "one gesture, one entry");
    let clip = node(&view, cx, FIRST);
    assert_eq!(clip["state"]["scope"], json!("shot:shot-1"));
    assert_eq!(clip["state"]["scope_label"], json!("Plano shot-1"));
    assert_eq!(clip["state"]["shot"], json!("shot-1"));
    assert_eq!(clip["state"]["start_frame"], json!(0));
    assert_eq!(
        clip["state"]["duration_frames"],
        json!(48),
        "the effect lasts 96 frames; the clip is cut at the end of the shot"
    );
    assert_eq!(field(&clip, "scope")["value"], json!("shot:shot-1"));
    assert_eq!(field(&clip, "scope")["at_default"], json!(false));
    // The shot holds it; the other one does not.
    let options = field(&clip, "scope")["options"].clone();
    assert_eq!(options[1]["applicable"], json!(true));
    assert_eq!(options[2]["applicable"], json!(false));
    assert_eq!(options[2]["reason"], json!("FX_SCOPE_MISMATCH"));

    // The monitor on the second shot: the clip lands there, at that frame.
    let frame = playhead_in_shot(&view, cx, "shot-2");
    assert!((48..96).contains(&frame));
    let depth = undo_depth(&view, cx);
    invoke(&view, cx, SECTION, "add_in_shot:fade").unwrap();
    assert_eq!(undo_depth(&view, cx), depth + 1);
    assert_eq!(scope_of(&view, cx, "fx-2"), FxScope::Shot("shot-2".into()));
    // The fade lasts 24 frames, and the shot has room for them or ends sooner.
    let (start, end) = clip_span(&view, cx, "fx-2");
    assert_eq!(start, frame);
    assert_eq!(end, (frame + 24).min(96));
    assert_eq!(
        node(&view, cx, "scene.fx.clip:fx-2")["state"]["shot"],
        json!("shot-2")
    );

    // The clips are in the scene's plan with the scope they hold, for the monitor and the export.
    let scopes = view.read_with(cx, |view, _| {
        let bundle = view.state.scenes.get("street").unwrap().scene.bundle();
        let at = |frame: i64| {
            bundle
                .fx
                .resolve(frame)
                .into_iter()
                .map(|pass| pass.scope)
                .collect::<Vec<_>>()
        };
        (at(0), at(frame))
    });
    assert_eq!(scopes.0, [FxScope::Shot("shot-1".into())]);
    assert_eq!(scopes.1, [FxScope::Shot("shot-2".into())]);
}

#[gpui_kit::test]
fn moving_a_clip_between_scopes_is_one_entry_each_and_undo_restores_it(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    invoke(&view, cx, SECTION, "add_in_shot:vignette").unwrap();
    let depth = undo_depth(&view, cx);

    // To the output: one entry, and the node says so.
    let back = invoke(&view, cx, FIRST, "set_scope:output").unwrap();
    assert_eq!(back["changed"], json!(true));
    assert_eq!(scope_of(&view, cx, "fx-1"), FxScope::Output);
    assert_eq!(undo_depth(&view, cx), depth + 1);
    let clip = node(&view, cx, FIRST);
    assert_eq!(clip["state"]["scope"], json!("output"));
    assert_eq!(field(&clip, "scope")["at_default"], json!(true));
    assert_eq!(
        action(&clip, "set_scope:shot:shot-1")["enabled"],
        json!(true)
    );
    assert_eq!(action(&clip, "set_scope:output")["enabled"], json!(false));

    // To the shot again, through `set` with the text of the scope: another entry.
    set(&view, cx, FIRST, "scope", Raw::Text("shot:shot-1".into())).unwrap();
    assert_eq!(scope_of(&view, cx, "fx-1"), FxScope::Shot("shot-1".into()));
    assert_eq!(undo_depth(&view, cx), depth + 2);

    // The scope the clip already holds is the core's no-op: nothing written, no entry.
    let same = set(&view, cx, FIRST, "scope", Raw::Text("shot:shot-1".into())).unwrap();
    assert_eq!(same["changed"], json!(false));
    assert_eq!(undo_depth(&view, cx), depth + 2);

    // The history is the shared one.
    view.update(cx, |view, cx| view.execute(Command::Undo, Source::Cli, cx))
        .unwrap();
    assert_eq!(scope_of(&view, cx, "fx-1"), FxScope::Output);
    view.update(cx, |view, cx| view.execute(Command::Undo, Source::Cli, cx))
        .unwrap();
    assert_eq!(scope_of(&view, cx, "fx-1"), FxScope::Shot("shot-1".into()));
    view.update(cx, |view, cx| view.execute(Command::Redo, Source::Cli, cx))
        .unwrap();
    assert_eq!(scope_of(&view, cx, "fx-1"), FxScope::Output);
}

#[gpui_kit::test]
fn a_refused_scope_is_typed_on_the_panel_and_writes_nothing(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    invoke(&view, cx, SECTION, "add_in_shot:vignette").unwrap();
    let depth = undo_depth(&view, cx);
    let before = revision(&view, cx);
    let span = clip_span(&view, cx, "fx-1");

    for (text, code) in [
        // The clip is `[0, 48)`: the second shot does not hold it, and the core never clamps it.
        ("shot:shot-2", "fx_scope_mismatch"),
        ("shot:nowhere", "unknown_shot"),
        ("node:anything", "fx_scope_not_yet"),
        ("banana", "invalid_value"),
        ("", "invalid_value"),
    ] {
        // Where the refusal points: the core names the shot it does not have as the id of its error,
        // as `unknown_clip` names the clip (`From<EditError>` turns an id into the target), so that is
        // the target of `unknown_shot`; every other refusal has no id and carries the clip's node.
        let target = if code == "unknown_shot" {
            "nowhere"
        } else {
            FIRST
        };
        let by_set = set(&view, cx, FIRST, "scope", Raw::Text(text.into())).unwrap_err();
        assert_eq!(by_set.code, code, "`{text}`: {by_set}");
        assert_eq!(by_set.target.as_deref(), Some(target), "`{text}`");
        assert_eq!(
            notice_code(&view, cx).as_deref(),
            Some(code),
            "`{text}`: the refusal is the panel's notice"
        );

        let by_action = invoke(&view, cx, FIRST, &format!("set_scope:{text}")).unwrap_err();
        assert_eq!(by_action.code, code, "`{text}` by invoke");
        assert_eq!(
            by_action.target.as_deref(),
            Some(target),
            "`{text}` by invoke"
        );

        assert_eq!(undo_depth(&view, cx), depth, "`{text}`");
        assert_eq!(revision(&view, cx), before, "`{text}`");
        assert_eq!(scope_of(&view, cx, "fx-1"), FxScope::Shot("shot-1".into()));
        assert_eq!(
            clip_span(&view, cx, "fx-1"),
            span,
            "`{text}`: nothing is clamped"
        );
    }

    // A scope is text: a number or a switch is not one.
    for raw in [Raw::Number(1.0), Raw::Bool(true)] {
        let error = set(&view, cx, FIRST, "scope", raw).unwrap_err();
        assert_eq!(error.code, "invalid_value");
    }
    assert_eq!(undo_depth(&view, cx), depth);
    // A clip that is not there.
    let missing = invoke(&view, cx, "scene.fx.clip:nope", "set_scope:output").unwrap_err();
    assert_eq!(missing.code, "unknown_element");

    // A clip whose frames leave its shot is judged by the same rule as the importer: retiming the
    // clip out of shot-1 is refused whole and writes nothing.
    let retime = set(&view, cx, FIRST, "duration_frames", Raw::Number(60.0));
    let error = retime.expect_err("a clip of shot-1 cannot last into shot-2");
    assert_eq!(error.code, "invalid_bundle", "{error}");
    assert_eq!(undo_depth(&view, cx), depth);
    assert_eq!(clip_span(&view, cx, "fx-1"), span);
}

#[gpui_kit::test]
fn the_selector_is_a_kit_select_whose_pick_is_the_same_command(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    invoke(&view, cx, SECTION, "add_in_shot:vignette").unwrap();
    let (select, items) = view
        .read_with(cx, |view, _| view.scope_select_of("street", "fx-1"))
        .expect("the panel paints a selector for the clip");
    // The values are the texts of the scopes; the words are the output, then each shot with the
    // frames it covers.
    assert_eq!(
        items
            .iter()
            .map(|item| (item.value.as_str(), item.title.as_ref()))
            .collect::<Vec<_>>(),
        [
            ("output", "Saída"),
            ("shot:shot-1", "Plano shot-1 · 0–47"),
            ("shot:shot-2", "Plano shot-2 · 48–95"),
        ]
    );
    let shown = |cx: &mut VisualTestContext| {
        select.read_with(cx, |select, _| select.selected_value().cloned())
    };
    let pick = |cx: &mut VisualTestContext, value: Option<&str>| {
        select.update(cx, |_, cx| {
            cx.emit(SelectEvent::Confirm(value.map(str::to_string)))
        });
        cx.run_until_parked();
    };
    assert_eq!(
        shown(cx).as_deref(),
        Some("shot:shot-1"),
        "it shows the scope the clip holds"
    );
    center_of(&view, cx, "control:fx-scope:fx-1");

    // The pick is the same `scene.fx.edit` a script sends: one entry.
    let depth = undo_depth(&view, cx);
    pick(cx, Some("output"));
    assert_eq!(scope_of(&view, cx, "fx-1"), FxScope::Output);
    assert_eq!(undo_depth(&view, cx), depth + 1);
    assert_eq!(shown(cx).as_deref(), Some("output"));
    assert_eq!(notice_code(&view, cx), None);

    // Picking what the clip holds is not an edit, and a cleared selection is no pick at all.
    pick(cx, Some("output"));
    pick(cx, None);
    assert_eq!(undo_depth(&view, cx), depth + 1);
    assert_eq!(notice_code(&view, cx), None);

    // A shot that does not hold the clip is refused: the typed notice, nothing written, and the
    // selector is back on the scope the document holds.
    pick(cx, Some("shot:shot-2"));
    assert_eq!(notice_code(&view, cx).as_deref(), Some("fx_scope_mismatch"));
    assert_eq!(undo_depth(&view, cx), depth + 1);
    assert_eq!(scope_of(&view, cx, "fx-1"), FxScope::Output);
    view.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    assert_eq!(
        shown(cx).as_deref(),
        Some("output"),
        "the select follows the document"
    );

    // The selector goes with its clip.
    invoke(&view, cx, FIRST, "remove").unwrap();
    view.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    assert!(
        view.read_with(cx, |view, _| view.scope_select_of("street", "fx-1"))
            .is_none()
    );
}

/// The panel on `street`, with a vignette of the first shot in it and in `yard`: `fx-1` is on
/// `shot:shot-1` in both scenes.
fn a_shot_clip_in_each_of_two_scenes(
    cx: &mut TestAppContext,
) -> (Entity<Workstation>, &mut VisualTestContext) {
    let (view, cx) = mounted(cx);
    invoke(&view, cx, SECTION, "add_in_shot:vignette").unwrap();
    attach_yard(&view, cx);
    choose(&view, cx, "yard");
    invoke(&view, cx, SECTION, "add_in_shot:vignette").unwrap();
    choose(&view, cx, "street");
    (view, cx)
}

#[gpui_kit::test]
fn a_pick_of_another_scene_writes_nothing_even_before_the_selector_is_dropped(
    cx: &mut TestAppContext,
) {
    let (view, cx) = a_shot_clip_in_each_of_two_scenes(cx);
    let (select, _) = view
        .read_with(cx, |view, _| view.scope_select_of("street", "fx-1"))
        .expect("the panel paints a selector for the clip");
    let depth = total_depth(&view, cx);

    // The pick is delivered in the update that switches the panel to `yard`: no paint has dropped
    // the selector of `street` yet, and `yard` has a clip of the same id.
    view.update(cx, |view, cx| {
        view.scene_choose("yard", cx).expect("an attached scene");
        select.update(cx, |_, cx| {
            cx.emit(SelectEvent::Confirm(Some("output".to_string())))
        });
    });
    cx.run_until_parked();
    assert_eq!(
        scope_in(&view, cx, "yard", "fx-1"),
        FxScope::Shot("shot-1".into()),
        "the clip of the scene the panel shows is not the one that was picked on"
    );
    assert_eq!(
        scope_in(&view, cx, "street", "fx-1"),
        FxScope::Shot("shot-1".into())
    );
    assert_eq!(total_depth(&view, cx), depth, "nothing was written");
    assert_eq!(notice_code(&view, cx), None);

    // The same pick on the scene it belongs to is an edit of that scene, and only of it.
    choose(&view, cx, "street");
    let (select, _) = view
        .read_with(cx, |view, _| view.scope_select_of("street", "fx-1"))
        .expect("the panel paints a selector for the clip");
    select.update(cx, |_, cx| {
        cx.emit(SelectEvent::Confirm(Some("output".to_string())))
    });
    cx.run_until_parked();
    assert_eq!(scope_in(&view, cx, "street", "fx-1"), FxScope::Output);
    assert_eq!(
        scope_in(&view, cx, "yard", "fx-1"),
        FxScope::Shot("shot-1".into())
    );
    assert_eq!(total_depth(&view, cx), depth + 1);
}

#[gpui_kit::test]
fn a_pick_of_a_clip_that_is_gone_writes_nothing_and_says_nothing(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    invoke(&view, cx, SECTION, "add_in_shot:vignette").unwrap();
    let (select, _) = view
        .read_with(cx, |view, _| view.scope_select_of("street", "fx-1"))
        .expect("the panel paints a selector for the clip");
    let depth = undo_depth(&view, cx);

    // The clip is removed and the pick arrives in the same update, before the paint drops the select.
    view.update(cx, |view, cx| {
        view.scene_invoke(FIRST, Some("remove"), cx)
            .expect("a scene node")
            .unwrap();
        select.update(cx, |_, cx| {
            cx.emit(SelectEvent::Confirm(Some("output".to_string())))
        });
    });
    cx.run_until_parked();
    assert_eq!(clips(&view, cx), 0);
    assert_eq!(undo_depth(&view, cx), depth + 1, "only the removal");
    assert_eq!(notice_code(&view, cx), None);
}

/// The core gives a new clip the smallest id that is free: a removed `fx-1` is the next clip's
/// `fx-1`. A pick made on the selector of the first one must not edit the second, which the user has
/// not seen; the selector painted for the second is live.
#[gpui_kit::test]
fn a_pick_of_a_clip_recreated_with_the_same_id_writes_nothing(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    invoke(&view, cx, SECTION, "add_in_shot:vignette").unwrap();
    let (select, _) = view
        .read_with(cx, |view, _| view.scope_select_of("street", "fx-1"))
        .expect("the panel paints a selector for the clip");
    let depth = undo_depth(&view, cx);

    // Removed and added again in the same update (no paint in between drops the selector), then the
    // pick of the old selector arrives.
    view.update(cx, |view, cx| {
        view.scene_invoke(FIRST, Some("remove"), cx)
            .expect("a scene node")
            .unwrap();
        view.scene_invoke(SECTION, Some("add_in_shot:vignette"), cx)
            .expect("a scene node")
            .unwrap();
        select.update(cx, |_, cx| {
            cx.emit(SelectEvent::Confirm(Some("output".to_string())))
        });
    });
    cx.run_until_parked();
    assert_eq!(clips(&view, cx), 1);
    assert_eq!(
        scope_of(&view, cx, "fx-1"),
        FxScope::Shot("shot-1".into()),
        "the new clip was not the one picked on"
    );
    assert_eq!(undo_depth(&view, cx), depth + 2, "the removal and the add");
    assert_eq!(notice_code(&view, cx), None);

    // The selector the panel paints for the new clip is the live one.
    view.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    let (select, _) = view
        .read_with(cx, |view, _| view.scope_select_of("street", "fx-1"))
        .expect("the panel paints a selector for the clip");
    select.update(cx, |_, cx| {
        cx.emit(SelectEvent::Confirm(Some("output".to_string())))
    });
    cx.run_until_parked();
    assert_eq!(scope_of(&view, cx, "fx-1"), FxScope::Output);
    assert_eq!(undo_depth(&view, cx), depth + 3);
}

#[gpui_kit::test]
fn an_unknown_action_lists_what_the_section_node_lists(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    let error = invoke(&view, cx, SECTION, "explode").unwrap_err();
    assert_eq!(error.code, "unknown_action");
    let advertised = error.details.unwrap()["actions"].clone();
    let listed = node(&view, cx, SECTION)["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|action| action["action"].clone())
        .collect::<Vec<_>>();
    assert_eq!(advertised, json!(listed));
    assert!(
        listed.contains(&json!("add_in_shot:vignette")),
        "the hint says what lays a clip on the shot: {listed:?}"
    );
}

#[gpui_kit::test]
fn the_duration_reset_of_a_clip_in_a_shot_is_what_the_shot_holds(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    // The vignette lasts 96 frames; the shot holds 48 of them.
    invoke(&view, cx, SECTION, "add_in_shot:vignette").unwrap();
    assert_eq!(clip_span(&view, cx, "fx-1"), (0, 48));
    let at_rest = field(&node(&view, cx, FIRST), "duration_frames");
    assert_eq!(at_rest["default"], json!(48.0), "{at_rest}");
    assert_eq!(at_rest["at_default"], json!(true));
    // The node publishes the limits of the timing fields the core computed for the clip's shot, not the
    // edit's: a clip that fills its shot has no room to start anywhere else, and no more to grow.
    let limits = |view: &Entity<Workstation>, cx: &mut VisualTestContext, name: &str| {
        let field = field(&node(view, cx, FIRST), name);
        (field["min"].clone(), field["max"].clone())
    };
    assert_eq!(limits(&view, cx, "start_frame"), (json!(0.0), json!(0.0)));
    assert_eq!(
        limits(&view, cx, "duration_frames"),
        (json!(1.0), json!(48.0))
    );

    set(&view, cx, FIRST, "duration_frames", Raw::Number(30.0)).unwrap();
    assert_eq!(clip_span(&view, cx, "fx-1"), (0, 30));
    // Shorter, it can start up to the frame that still lets it end inside the shot (48 - 30).
    assert_eq!(limits(&view, cx, "start_frame"), (json!(0.0), json!(18.0)));
    assert_eq!(
        limits(&view, cx, "duration_frames"),
        (json!(1.0), json!(48.0))
    );
    let moved = node(&view, cx, FIRST);
    assert_eq!(field(&moved, "duration_frames")["at_default"], json!(false));
    assert_eq!(
        action(&moved, "reset:duration_frames")["enabled"],
        json!(true)
    );

    // The reset is an op the edit accepts, and it restores what the add gave.
    let depth = undo_depth(&view, cx);
    invoke(&view, cx, FIRST, "reset:duration_frames").unwrap();
    assert_eq!(clip_span(&view, cx, "fx-1"), (0, 48));
    assert_eq!(undo_depth(&view, cx), depth + 1);
    assert_eq!(notice_code(&view, cx), None);
}

#[gpui_kit::test]
fn the_add_buttons_on_the_shot_run_the_shared_handler_and_are_disabled_outside_the_edit(
    cx: &mut TestAppContext,
) {
    let (view, cx) = mounted(cx);
    let depth = undo_depth(&view, cx);
    let add = center_of(&view, cx, "control:fx-add-shot:vignette");
    cx.simulate_click(add, Default::default());
    cx.run_until_parked();
    assert_eq!(clips(&view, cx), 1);
    assert_eq!(undo_depth(&view, cx), depth + 1);
    assert_eq!(scope_of(&view, cx, "fx-1"), FxScope::Shot("shot-1".into()));
    assert_eq!(clip_span(&view, cx, "fx-1"), (0, 48));
    // The button is in the section's header; so is the plain one, which keeps the output.
    let plain = center_of(&view, cx, "control:fx-add:fade");
    cx.simulate_click(plain, Default::default());
    cx.run_until_parked();
    assert_eq!(scope_of(&view, cx, "fx-2"), FxScope::Output);

    // The monitor past the edit shows no shot: the node says why, nothing is added by the action
    // and the painted button does nothing either.
    playhead_outside_the_edit(&view, cx);
    let section = node(&view, cx, SECTION);
    assert_eq!(section["state"]["shot_here"], Value::Null);
    let off = action(&section, "add_in_shot:vignette");
    assert_eq!(off["enabled"], json!(false));
    assert!(off["disabled_reason"].is_string());
    let depth = undo_depth(&view, cx);
    let error = invoke(&view, cx, SECTION, "add_in_shot:vignette").unwrap_err();
    assert_eq!(error.code, "disabled_element");
    assert_eq!(error.target.as_deref(), Some(SECTION));
    assert_eq!(notice_code(&view, cx).as_deref(), Some("disabled_element"));
    assert_eq!(clips(&view, cx), 2);
    assert_eq!(undo_depth(&view, cx), depth);
    let inert = center_of(&view, cx, "control:fx-add-shot:vignette");
    cx.simulate_click(inert, Default::default());
    cx.run_until_parked();
    assert_eq!(clips(&view, cx), 2);
    assert_eq!(undo_depth(&view, cx), depth);

    // An effect the catalog does not have is the core's refusal, with the shot found.
    playhead_in_shot(&view, cx, "shot-1");
    let unknown = invoke(&view, cx, SECTION, "add_in_shot:sepia").unwrap_err();
    assert_eq!(unknown.code, "unknown_effect", "{unknown}");
    assert_eq!(clips(&view, cx), 2);
}

#[gpui_kit::test]
fn the_same_effects_are_offered_on_a_shot_as_on_the_output_with_the_same_availability(
    cx: &mut TestAppContext,
) {
    // The catalog of this build has only looks, so every offer is enabled here; a family the layer
    // does not draw (transitions, P4b) would be listed disabled with the code the plain add button
    // uses, and the comparison below would hold for it too.
    let (view, cx) = mounted(cx);
    let section = node(&view, cx, SECTION);
    for entry in section["actions"].as_array().unwrap() {
        let name = entry["action"].as_str().unwrap();
        if let Some(effect) = name.strip_prefix("add_in_shot:") {
            let plain = action(&section, &format!("add:{effect}"));
            assert_eq!(
                (entry["enabled"].clone(), entry["disabled_reason"].is_null()),
                (plain["enabled"].clone(), plain["disabled_reason"].is_null()),
                "`{effect}`: the same effects are offered on a shot as on the output"
            );
        }
    }
}
