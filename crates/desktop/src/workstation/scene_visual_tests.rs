//! BL087 (frente R9): the layout of the scene inspector at the sizes the window really takes.
//!
//! The dock lets a person make the inspector anywhere from 100 to 628 px wide and the window
//! as small as 960x640, and the native captures of `scripts/scene-visual-proof.py` showed what
//! the default dock hid: slider tracks squeezed to nothing, long values printed over the track,
//! a tree cut in half and never scrolled to the selection, a tree stuck at 240 px in a tall
//! window. These tests keep each of those fixed by measuring the geometry probes of a real
//! (test-platform) layout, and the pointer test shows a drag still ends in ONE undo entry in the
//! stacked layout.
//!
//! What they are NOT: pixels. GPUI's test platform lays out and routes input but paints nothing,
//! so colours, glyphs and the native scrollbar are judged only by the screenshots that script
//! takes. Wheel events here are injected, not a physical device (BL043).
//!
//! Declared from `scene_tests.rs` (`#[path]`) so that no shared `mod.rs` line is needed.
use super::super::input_tests::fixture;
use super::super::scene_gesture::SliderKey;
use super::super::scene_model::{MAX_VALUE, MIN_TRACK, TREE_ROW_HEIGHT};
use super::super::scene_panel::{AttachSceneOpener, attach_availability};
use super::super::scene_ui::InspectorMode;
use super::super::*;
use super::{drag_to, press, release, undo_depth};
// The Kit's test macro expands to `#[test]`; the glob import above would resolve it to the Kit macro again.
use core::prelude::v1::test;
use gpui_kit::base::ScrollbarHandle;
use serde_json::json;

use tessel_editor_core::scene_edit::{PropertyRef, PropertyValue, read_property};
use tessel_editor_core::scene_interaction::SelectMode;
use tessel_editor_core::scene_project::{SceneCommand, SceneSource};

use crate::state::layout::Region;

const RIG: &str = include_str!(
    "../../../../docs/migration/fixtures/bundles/valid-graph-sockets-performance.bundle.json"
);

const SCENE: &str = "rig";
const PEN: &str = "scene/scene-rig/node/pen";
const WIDTH: &str = "scene/scene-rig/node/pen/width";
const POSITION: &str = "scene/scene-rig/node/pen/position";
/// The first entity listed below the 8 rows a 960x640 window's tree shows.
const LAST_CROWD: &str = "scene/scene-rig/node/crowd-23";
const DEEPEST: &str = "scene/scene-rig/node/deep-3";
const CROWD: usize = 24;
const DEEP: usize = 4;

/// The graph fixture with a chain of nested groups under `arm` (collapsed by default) and a wide
/// level under `stage`, so the tree is both deeper and longer than any panel.
fn crowded_bundle() -> String {
    let mut bundle: Value = serde_json::from_str(RIG).unwrap();
    let nodes = bundle["sceneDefinitions"][0]["nodes"]
        .as_array_mut()
        .unwrap();
    let group = |id: String, parent: String| {
        json!({
            "id": id, "parentId": parent, "type": "group",
            "localPosition": { "x": 0, "y": 0, "z": 0 },
            "localRotation": { "x": 0, "y": 0, "z": 0, "w": 1 },
            "localScale": { "x": 1, "y": 1, "z": 1 },
        })
    };
    let mut parent = "arm".to_string();
    for level in 0..DEEP {
        let id = format!("deep-{level}");
        nodes.push(group(id.clone(), parent));
        parent = id;
    }
    for index in 0..CROWD {
        nodes.push(group(format!("crowd-{index:02}"), "stage".to_string()));
    }
    bundle.to_string()
}

fn run(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    command: SceneCommand,
) -> Result<Value, CommandError> {
    let result = view.update(cx, |view, cx| {
        view.execute(Command::Scene(command), Source::Cli, cx)
    });
    settle(cx);
    result
}

/// Lets the window draw, then delivers the extra frame a resize asks for (`watch_inspector_size`),
/// until the layout stops changing. The test platform has no frame loop of its own.
fn settle(cx: &mut VisualTestContext) {
    for _ in 0..4 {
        cx.run_until_parked();
        cx.update(|window, cx| window.simulate_next_frame(cx));
    }
    cx.run_until_parked();
}

fn expand(view: &Entity<Workstation>, cx: &mut VisualTestContext, id: &str) {
    view.update(cx, |view, cx| {
        view.scene_invoke(
            &format!("scene.entity:scene/scene-rig/node/{id}"),
            Some("expand"),
            cx,
        )
        .expect("a scene entity node")
        .unwrap();
    });
}

/// 960x640 window, the rig scene attached, the scene inspector showing, the tree open down to
/// `arm`, nothing selected.
fn mounted(cx: &mut TestAppContext) -> (Entity<Workstation>, &mut VisualTestContext) {
    let (view, cx) = fixture(cx);
    cx.simulate_resize(size(px(960.), px(640.)));
    run(
        &view,
        cx,
        SceneCommand::Attach {
            id: Some(SCENE.into()),
            name: None,
            source: SceneSource::Text(crowded_bundle()),
        },
    )
    .unwrap();
    view.update(cx, |view, cx| view.scene_set_mode(InspectorMode::Scene, cx));
    for id in ["root", "stage", "arm"] {
        expand(&view, cx, id);
    }
    settle(cx);
    (view, cx)
}

fn select(view: &Entity<Workstation>, cx: &mut VisualTestContext, path: &str) {
    run(
        view,
        cx,
        SceneCommand::Select {
            scene: Some(SCENE.into()),
            entities: vec![path.to_string()],
            mode: SelectMode::Replace,
        },
    )
    .unwrap();
}

fn set_inspector_width(view: &Entity<Workstation>, cx: &mut VisualTestContext, width: f32) {
    view.update_in(cx, |view, window, cx| {
        view.execute_layout(
            LayoutCommand::ResizeRegion {
                region: Region::Right,
                size: width,
            },
            Source::Cli,
            window,
            cx,
        )
    })
    .unwrap();
    settle(cx);
}

fn bounds(view: &Entity<Workstation>, cx: &mut VisualTestContext, id: &str) -> Bounds<Pixels> {
    view.read_with(cx, |view, _| view.geometry.get(id))
        .unwrap_or_else(|| panic!("no probe `{id}` was painted"))
}

fn key(path: &str, index: usize) -> SliderKey {
    SliderKey {
        scene: SCENE.into(),
        entity: PEN.into(),
        path: path.into(),
        index,
    }
}

fn control(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    key: &SliderKey,
) -> Bounds<Pixels> {
    bounds(view, cx, &format!("control:{}", key.element_id()))
}

fn value_cell(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    key: &SliderKey,
) -> Bounds<Pixels> {
    bounds(view, cx, &format!("value:{}", key.element_id()))
}

/// Every slider the pen shows: position, rotation and scale (X/Y/Z), width and height.
fn pen_keys() -> Vec<SliderKey> {
    let mut keys = Vec::new();
    for property in ["position", "rotation", "scale"] {
        for index in 0..3 {
            keys.push(key(&format!("{PEN}/{property}"), index));
        }
    }
    keys.push(key(WIDTH, 0));
    keys.push(key(&format!("{PEN}/height"), 0));
    keys
}

/// The first row the tree is scrolled to, from the scroll offset (the Kit list does not register
/// child bounds, so GPUI's own `logical_scroll_top_index` stays at 0 for it).
fn tree_top_row(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> usize {
    view.read_with(cx, |view, cx| {
        let offset = view.scene_ui.tree.read(cx).scroll_handle().offset().y;
        (f32::from(-offset) / TREE_ROW_HEIGHT).round() as usize
    })
}

fn scroll(cx: &mut VisualTestContext, at: Point<Pixels>, dy: f32) {
    cx.simulate_event(ScrollEvent {
        position: at,
        delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
        ..Default::default()
    });
    settle(cx);
}

/// What a person does with a slider below the fold: wheels the inspector until it shows. A
/// pointer cannot press what the scroll area clips. The wheel goes over plain text (the
/// attach reason): over the tree it would scroll the tree instead.
fn bring_into_view(view: &Entity<Workstation>, cx: &mut VisualTestContext, key: &SliderKey) {
    let inspector = bounds(view, cx, "region:inspector");
    if control(view, cx, key).bottom() <= inspector.bottom() {
        return;
    }
    let plain_text = bounds(view, cx, "scene:attach-reason").center();
    scroll(cx, plain_text, -3000.0);
    let track = control(view, cx, key);
    assert!(
        track.top() >= inspector.top() && track.bottom() <= inspector.bottom(),
        "{} is still out of view after wheeling to the end: {track:?} in {inspector:?}",
        key.element_id()
    );
}

fn wide(a: Bounds<Pixels>) -> f32 {
    f32::from(a.size.width)
}

fn high(a: Bounds<Pixels>) -> f32 {
    f32::from(a.size.height)
}

#[gpui_kit::test]
fn the_tree_shows_whole_rows_and_grows_with_the_window(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    let small = bounds(&view, cx, "scene:tree");
    let rows = high(small) / TREE_ROW_HEIGHT;
    assert_eq!(rows.fract(), 0.0, "no row is cut in half: {rows} rows");
    assert!(rows >= 5.0, "the tree keeps at least five rows: {rows}");
    let inspector = bounds(&view, cx, "region:inspector");
    assert!(
        high(small) <= high(inspector) * 0.42 + 0.5,
        "the tree leaves the properties their room: {} of {}",
        high(small),
        high(inspector)
    );

    cx.simulate_resize(size(px(1440.), px(1000.)));
    settle(cx);
    let tall = bounds(&view, cx, "scene:tree");
    assert!(
        high(tall) > high(small),
        "a taller window shows more of the tree ({} -> {})",
        high(small),
        high(tall)
    );
    assert_eq!((high(tall) / TREE_ROW_HEIGHT).fract(), 0.0);
}

#[gpui_kit::test]
fn every_slider_keeps_a_draggable_track_inside_the_inspector_at_every_dock_width(
    cx: &mut TestAppContext,
) {
    let (view, cx) = mounted(cx);
    select(&view, cx, PEN);
    for width in [100.0, 150.0, 180.0, 231.0, 232.0, 272.0, 400.0, 628.0] {
        set_inspector_width(&view, cx, width);
        let inspector = bounds(&view, cx, "region:inspector");
        assert!(
            (wide(inspector) - width).abs() < 1.0,
            "the dock gave {} for {width}",
            wide(inspector)
        );
        for key in pen_keys() {
            let track = control(&view, cx, &key);
            assert!(
                wide(track) >= MIN_TRACK,
                "{} at {width} px: the track is {} px",
                key.element_id(),
                wide(track)
            );
            assert!(
                track.left() >= inspector.left() - px(0.5)
                    && track.right() <= inspector.right() + px(0.5),
                "{} at {width} px: track {:?} leaves the inspector {:?}",
                key.element_id(),
                track,
                inspector
            );
        }
    }
}

#[gpui_kit::test]
fn a_long_value_never_covers_its_track_in_either_layout(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    select(&view, cx, PEN);
    view.update(cx, |view, cx| {
        view.scene_set_component(POSITION, 0, 12345.678, Source::Cli, cx)
    })
    .unwrap();
    settle(cx);
    let x = key(POSITION, 0);

    set_inspector_width(&view, cx, 272.0);
    let (track, value) = (control(&view, cx, &x), value_cell(&view, cx, &x));
    assert!(
        value.left() >= track.right() - px(0.5),
        "one-line row: the value {value:?} starts inside the track {track:?}"
    );
    assert!(
        wide(value) <= MAX_VALUE + 0.5,
        "the value column is capped: {}",
        wide(value)
    );

    set_inspector_width(&view, cx, 120.0);
    let (track, value) = (control(&view, cx, &x), value_cell(&view, cx, &x));
    assert!(
        value.bottom() <= track.top() + px(0.5),
        "stacked row: the value {value:?} sits over the track {track:?}"
    );
    assert!(wide(value) <= MAX_VALUE + 0.5);
}

#[gpui_kit::test]
fn a_narrow_inspector_stacks_the_slider_and_a_wide_one_keeps_one_line(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    select(&view, cx, PEN);
    let x = key(POSITION, 0);

    set_inspector_width(&view, cx, 272.0);
    let (track, value) = (control(&view, cx, &x), value_cell(&view, cx, &x));
    assert!(
        (track.center().y - value.center().y).abs() < px(6.),
        "default dock: the value is on the track's line ({track:?} {value:?})"
    );

    set_inspector_width(&view, cx, 100.0);
    let (track, value) = (control(&view, cx, &x), value_cell(&view, cx, &x));
    assert!(
        track.top() >= value.bottom() - px(0.5),
        "dock floor: the track takes its own line ({track:?} {value:?})"
    );
    assert!(wide(track) >= MIN_TRACK, "{}", wide(track));
}

#[gpui_kit::test]
fn selecting_a_hidden_entity_opens_its_ancestors_and_scrolls_to_its_row(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    assert_eq!(tree_top_row(&view, cx), 0);
    assert!(
        view.read_with(cx, |view, cx| view
            .tree_node(&format!("scene.entity:{DEEPEST}"), cx))
            .is_none(),
        "the chain starts collapsed, so the deepest entity is not listed"
    );

    // What a socket or headless client does: select by ID, with no click on a row.
    select(&view, cx, DEEPEST);
    let selected = view.read_with(cx, |view, cx| {
        view.scene_ui
            .tree
            .read(cx)
            .selected_item()
            .map(|item| item.id.to_string())
    });
    assert_eq!(
        selected.as_deref(),
        Some(DEEPEST),
        "the tree shows the selection"
    );
    let listed = view
        .read_with(cx, |view, cx| {
            view.tree_node(&format!("scene.entity:{DEEPEST}"), cx)
        })
        .expect("its ancestors were opened");
    assert_eq!(listed["state"]["selected"], true);
    let parent = view
        .read_with(cx, |view, cx| {
            view.tree_node("scene.entity:scene/scene-rig/node/deep-0", cx)
        })
        .unwrap();
    assert_eq!(parent["state"]["expanded"], true);

    // A row below the visible ones: the tree scrolls to it.
    select(&view, cx, LAST_CROWD);
    assert!(
        tree_top_row(&view, cx) > 0,
        "the tree stayed at the top with {LAST_CROWD} selected"
    );
    // The last row is flush with the bottom of the box only if every row is `TREE_ROW_HEIGHT`
    // tall: the pitch the list really lays out, not the one the model assumes.
    let tree = bounds(&view, cx, "scene:tree");
    let (offset, last) = view.read_with(cx, |view, cx| {
        let state = view.scene_ui.tree.read(cx);
        (
            f32::from(-state.scroll_handle().offset().y),
            state.selected_index().unwrap(),
        )
    });
    assert_eq!(
        (high(tree) + offset) / (last + 1) as f32,
        TREE_ROW_HEIGHT,
        "the rows are not {TREE_ROW_HEIGHT} px apart"
    );
}

#[gpui_kit::test]
fn the_tree_is_revealed_once_so_a_later_edit_does_not_pull_it_back(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    select(&view, cx, LAST_CROWD);
    assert!(tree_top_row(&view, cx) > 0);

    // The person scrolls the tree away to look at something else...
    view.read_with(cx, |view, cx| {
        view.scene_ui
            .tree
            .read(cx)
            .scroll_handle()
            .scroll_to_item(0, ScrollStrategy::Top)
    });
    settle(cx);
    assert_eq!(tree_top_row(&view, cx), 0);

    // ...and an edit of the selected entity repaints the panel with the same selection.
    view.update(cx, |view, cx| {
        view.scene_set_component(&format!("{LAST_CROWD}/position"), 0, 0.5, Source::Cli, cx)
    })
    .unwrap();
    settle(cx);
    assert_eq!(tree_top_row(&view, cx), 0, "the edit moved the tree");
}

#[gpui_kit::test]
fn the_wheel_scrolls_the_tree_over_the_tree_and_the_inspector_elsewhere(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    select(&view, cx, PEN);

    let tree = bounds(&view, cx, "scene:tree");
    assert_eq!(tree_top_row(&view, cx), 0);
    scroll(cx, tree.center(), -200.0);
    assert!(
        tree_top_row(&view, cx) > 0,
        "the wheel over the tree moved the tree"
    );

    // The last slider is below the fold of a 960x640 window; the wheel over the properties
    // brings it up.
    let last = key(&format!("{PEN}/height"), 0);
    let before = control(&view, cx, &last);
    let inspector = bounds(&view, cx, "region:inspector");
    assert!(
        before.top() > inspector.bottom(),
        "the fixture must overflow the panel: {before:?} in {inspector:?}"
    );
    let over_properties = control(&view, cx, &key(POSITION, 0)).center();
    scroll(cx, over_properties, -400.0);
    let after = control(&view, cx, &last);
    assert!(
        after.top() < before.top() - px(100.),
        "the inspector did not scroll: {before:?} -> {after:?}"
    );
}

#[gpui_kit::test]
fn a_disabled_control_prints_why_on_screen(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    let inspector = bounds(&view, cx, "region:inspector");
    for id in ["scene:monitor-reason", "scene:attach-reason"] {
        let reason = bounds(&view, cx, id);
        assert!(
            high(reason) > 0.0 && wide(reason) > 0.0,
            "`{id}` has no size: {reason:?}"
        );
        assert!(
            reason.left() >= inspector.left() - px(0.5)
                && reason.right() <= inspector.right() + px(0.5),
            "`{id}` leaves the inspector: {reason:?}"
        );
    }
    // Narrowest dock: the reasons wrap instead of running out of the panel, and the button is
    // whole (the full label did not fit and ran past the edge).
    set_inspector_width(&view, cx, 100.0);
    let inspector = bounds(&view, cx, "region:inspector");
    let button = bounds(&view, cx, "scene:attach");
    assert!(
        button.left() >= inspector.left() && button.right() <= inspector.right(),
        "the attach button {button:?} leaves the inspector {inspector:?}"
    );
    for id in ["scene:monitor-reason", "scene:attach-reason"] {
        let reason = bounds(&view, cx, id);
        assert!(
            reason.right() <= inspector.right() + px(0.5),
            "`{id}`: {reason:?}"
        );
    }
}

fn marker(_: &mut Workstation, _: &mut Window, _: &mut Context<Workstation>) {}

#[test]
fn attach_is_offered_only_with_a_dialog_to_open_and_says_why_otherwise() {
    let reason = attach_availability(None).unwrap_err();
    assert!(reason.contains("scene.attach"), "{reason}");
    let opener = attach_availability(Some(marker)).unwrap();
    assert!(std::ptr::fn_addr_eq(opener, marker as AttachSceneOpener));
}

#[gpui_kit::test]
fn dragging_a_slider_in_the_stacked_layout_is_one_gesture_and_one_undo_entry(
    cx: &mut TestAppContext,
) {
    let (view, cx) = mounted(cx);
    select(&view, cx, PEN);
    set_inspector_width(&view, cx, 120.0);
    let width = key(WIDTH, 0);
    bring_into_view(&view, cx, &width);
    let before = pen_width(&view, cx);
    let entries = undo_depth(&view, cx);

    let track = control(&view, cx, &width);
    let at = |fraction: f32| point(track.left() + track.size.width * fraction, track.center().y);
    press(cx, at(0.5));
    for fraction in [0.6, 0.7, 0.8] {
        drag_to(cx, at(fraction));
    }
    assert!(
        view.read_with(cx, |view, _| view
            .state
            .scene_interaction
            .gesture()
            .is_some()),
        "the drag opened a gesture"
    );
    assert_eq!(
        pen_width(&view, cx),
        before,
        "the document is untouched mid-drag"
    );
    assert_eq!(undo_depth(&view, cx), entries);
    release(cx, at(0.8));
    cx.run_until_parked();

    let after = pen_width(&view, cx);
    assert!(after > before, "{before} -> {after}");
    assert_eq!(
        undo_depth(&view, cx),
        entries + 1,
        "the whole drag is one entry"
    );
    view.update(cx, |view, cx| view.execute(Command::Undo, Source::Cli, cx))
        .unwrap();
    assert_eq!(pen_width(&view, cx), before);
}

fn pen_width(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> f64 {
    view.read_with(cx, |view, _| {
        let attached = view.state.scenes.get(SCENE).unwrap();
        match read_property(&attached.scene, &PropertyRef::parse(WIDTH).unwrap()).unwrap() {
            PropertyValue::Number(width) => width,
            other => panic!("width is a number, got {other:?}"),
        }
    })
}

#[gpui_kit::test]
fn the_wheel_over_a_slider_still_scrolls_the_inspector(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    select(&view, cx, PEN);
    let last = key(WIDTH, 0);
    for width in [120.0, 272.0] {
        set_inspector_width(&view, cx, width);
        bring_into_view(&view, cx, &last);
        let over = control(&view, cx, &last);
        // Up (positive delta) from the end: the slider under the pointer moves down the screen.
        scroll(cx, over.center(), 60.0);
        let after = control(&view, cx, &last);
        assert!(
            after.top() > over.top(),
            "{width} px: the wheel over the slider did not scroll ({over:?} -> {after:?})"
        );
    }
}
