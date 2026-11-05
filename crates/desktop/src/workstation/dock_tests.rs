//! The dock is exercised through the same entry point the CLI and the header
//! buttons use (`execute_layout`), against the live `DockArea` rendered in a
//! headless window. Nothing here edits the pure `Layout` directly.
use serde_json::Value;

use super::input_tests::fixture;
use super::*;
// The Kit's test macro expands to `#[test]`; the glob import above would resolve it to the Kit macro again.
use crate::state::layout::{Destination, GroupRef, Node, Region, Side};
use core::prelude::v1::test;

fn run(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    command: LayoutCommand,
) -> Result<Value, CommandError> {
    let result = view.update_in(cx, |view, window, cx| {
        view.execute_layout(command, Source::Cli, window, cx)
    });
    cx.run_until_parked();
    result
}

fn snapshot(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Layout {
    view.read_with(cx, |view, cx| view.layout_snapshot(cx))
}

fn region_of(layout: &Layout, panel: PanelKind) -> Option<Region> {
    layout.locate(panel).map(|placement| placement.region)
}

#[gpui_kit::test]
fn opens_with_library_left_monitor_center_timeline_below_inspector_right(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let layout = snapshot(&view, cx);
    assert_eq!(region_of(&layout, PanelKind::Library), Some(Region::Left));
    assert_eq!(region_of(&layout, PanelKind::Monitor), Some(Region::Center));
    assert_eq!(
        region_of(&layout, PanelKind::Timeline),
        Some(Region::Bottom)
    );
    assert_eq!(
        region_of(&layout, PanelKind::Inspector),
        Some(Region::Right)
    );
    assert!(layout.hidden().is_empty());
    // One monitor only: the center holds exactly that panel.
    let center = layout.region(Region::Center).unwrap();
    assert_eq!(
        center.root.to_json()["children"][0]["panels"],
        json!(["panel:monitor"])
    );
}

#[gpui_kit::test]
fn hiding_and_showing_a_panel_edits_the_live_area(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    run(
        &view,
        cx,
        LayoutCommand::SetVisible {
            panel: PanelKind::Inspector,
            visible: false,
        },
    )
    .unwrap();
    let layout = snapshot(&view, cx);
    assert_eq!(layout.hidden(), vec![PanelKind::Inspector]);
    assert!(!layout.is_visible(PanelKind::Inspector));

    run(
        &view,
        cx,
        LayoutCommand::SetVisible {
            panel: PanelKind::Inspector,
            visible: true,
        },
    )
    .unwrap();
    let layout = snapshot(&view, cx);
    assert!(layout.hidden().is_empty());
    assert_eq!(
        region_of(&layout, PanelKind::Inspector),
        Some(Region::Right)
    );
}

#[gpui_kit::test]
fn moving_a_panel_between_docks_follows_the_live_area(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    run(
        &view,
        cx,
        LayoutCommand::Move {
            panel: PanelKind::Library,
            to: Destination::Region(Region::Right),
        },
    )
    .unwrap();
    let layout = snapshot(&view, cx);
    assert_eq!(region_of(&layout, PanelKind::Library), Some(Region::Right));
    assert_eq!(
        region_of(&layout, PanelKind::Inspector),
        Some(Region::Right)
    );
}

#[gpui_kit::test]
fn a_panel_can_join_another_panels_tab_group(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let target = snapshot(&view, cx)
        .resolve_group(&GroupRef::HoldingPanel(PanelKind::Inspector))
        .unwrap();
    run(
        &view,
        cx,
        LayoutCommand::Move {
            panel: PanelKind::Library,
            to: Destination::Group {
                group: target,
                index: Some(0),
            },
        },
    )
    .unwrap();
    let layout = snapshot(&view, cx);
    let library = layout.locate(PanelKind::Library).unwrap();
    let inspector = layout.locate(PanelKind::Inspector).unwrap();
    assert_eq!(library.group, inspector.group);
    assert_eq!(library.index, 0);
}

#[gpui_kit::test]
fn splitting_beside_a_group_creates_a_resizable_split(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let target = snapshot(&view, cx)
        .resolve_group(&GroupRef::HoldingPanel(PanelKind::Inspector))
        .unwrap();
    run(
        &view,
        cx,
        LayoutCommand::Move {
            panel: PanelKind::Library,
            to: Destination::Beside {
                group: target,
                side: Side::Top,
            },
        },
    )
    .unwrap();
    let layout = snapshot(&view, cx);
    assert_eq!(region_of(&layout, PanelKind::Library), Some(Region::Right));
    let split = layout
        .split_ids()
        .into_iter()
        .find(|id| {
            matches!(
                layout.split(*id),
                Some((Region::Right, Node::Split { children, .. })) if children.len() == 2
            )
        })
        .expect("the right dock must now hold a two-pane split");

    // Sizes must keep the split's total, so the second pane takes the remainder.
    let Some((_, Node::Split { sizes: before, .. })) = layout.split(split) else {
        panic!("not a split");
    };
    let total: f32 = before.iter().sum();
    run(
        &view,
        cx,
        LayoutCommand::ResizeSplit {
            split,
            sizes: vec![240.0, total - 240.0],
        },
    )
    .unwrap();
    let layout = snapshot(&view, cx);
    let Some((_, Node::Split { sizes, .. })) = layout.split(split) else {
        panic!("the split must survive a resize");
    };
    assert_eq!(sizes.len(), 2);
    assert!((sizes[0] - 240.0).abs() < 1.0, "{sizes:?}");
}

#[gpui_kit::test]
fn dock_size_and_collapse_are_driven_by_commands(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    run(
        &view,
        cx,
        LayoutCommand::ResizeRegion {
            region: Region::Left,
            size: 320.0,
        },
    )
    .unwrap();
    let left = snapshot(&view, cx).region(Region::Left).cloned().unwrap();
    assert_eq!(left.size, Some(320.0));

    run(
        &view,
        cx,
        LayoutCommand::SetRegionOpen {
            region: Region::Left,
            open: false,
        },
    )
    .unwrap();
    let layout = snapshot(&view, cx);
    assert!(!layout.region(Region::Left).unwrap().open);
    // A collapsed dock keeps its panels placed but takes them off screen.
    assert!(layout.locate(PanelKind::Library).is_some());
    assert!(!layout.is_visible(PanelKind::Library));
}

#[gpui_kit::test]
fn invalid_commands_report_structured_errors_and_change_nothing(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let before = snapshot(&view, cx).to_json();
    let cases = [
        LayoutCommand::Move {
            panel: PanelKind::Library,
            to: Destination::Group {
                group: 9999,
                index: None,
            },
        },
        LayoutCommand::ResizeRegion {
            region: Region::Center,
            size: 300.0,
        },
        LayoutCommand::ResizeRegion {
            region: Region::Left,
            size: 5.0,
        },
        LayoutCommand::ResizeSplit {
            split: 9999,
            sizes: vec![100.0],
        },
        LayoutCommand::Load {
            json: "not json".into(),
        },
    ];
    for command in cases {
        let error = run(&view, cx, command.clone()).unwrap_err();
        assert!(!error.code.is_empty(), "{command:?}");
        assert_eq!(snapshot(&view, cx).to_json(), before, "{command:?}");
    }
}

#[gpui_kit::test]
fn reset_restores_the_default_layout(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let before = snapshot(&view, cx).to_json();
    run(
        &view,
        cx,
        LayoutCommand::Move {
            panel: PanelKind::Timeline,
            to: Destination::Region(Region::Left),
        },
    )
    .unwrap();
    run(
        &view,
        cx,
        LayoutCommand::SetVisible {
            panel: PanelKind::Library,
            visible: false,
        },
    )
    .unwrap();
    assert_ne!(snapshot(&view, cx).to_json(), before);
    run(&view, cx, LayoutCommand::Reset).unwrap();
    assert_eq!(
        region_of(&snapshot(&view, cx), PanelKind::Timeline),
        Some(Region::Bottom)
    );
    assert!(snapshot(&view, cx).hidden().is_empty());
}

#[gpui_kit::test]
fn a_dumped_layout_loads_back_into_the_live_area(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    run(
        &view,
        cx,
        LayoutCommand::Move {
            panel: PanelKind::Inspector,
            to: Destination::Region(Region::Left),
        },
    )
    .unwrap();
    let saved = view.read_with(cx, |view, cx| view.dock_report(cx))["dump"].to_string();
    run(&view, cx, LayoutCommand::Reset).unwrap();
    assert_eq!(
        region_of(&snapshot(&view, cx), PanelKind::Inspector),
        Some(Region::Right)
    );
    run(&view, cx, LayoutCommand::Load { json: saved }).unwrap();
    assert_eq!(
        region_of(&snapshot(&view, cx), PanelKind::Inspector),
        Some(Region::Left)
    );
}

#[gpui_kit::test]
fn the_cli_reports_what_the_ui_shows(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    run(
        &view,
        cx,
        LayoutCommand::SetVisible {
            panel: PanelKind::Library,
            visible: false,
        },
    )
    .unwrap();
    let nodes = view.read_with(cx, |view, cx| view.tree_nodes(cx));
    let library = nodes
        .iter()
        .find(|node| node["id"] == "panel:library")
        .expect("hidden panels stay addressable so they can be reopened");
    assert_eq!(library["visible"], false);
    assert_eq!(library["state"]["visible"], false);
    view.read_with(cx, |view, _| {
        let activity = view.activity.as_ref().expect("the command leaves a trace");
        assert!(matches!(activity.source, Source::Cli));
        assert!(!activity.error);
    });
}

#[gpui_kit::test]
fn a_dock_emptied_by_a_move_collapses_and_reopens_when_a_panel_returns(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    run(
        &view,
        cx,
        LayoutCommand::Move {
            panel: PanelKind::Library,
            to: Destination::Region(Region::Right),
        },
    )
    .unwrap();
    let layout = snapshot(&view, cx);
    assert!(
        !layout.region(Region::Left).unwrap().open,
        "an empty left dock must not stay open as a blank strip"
    );
    assert!(layout.region(Region::Right).unwrap().open);

    run(
        &view,
        cx,
        LayoutCommand::Move {
            panel: PanelKind::Library,
            to: Destination::Region(Region::Left),
        },
    )
    .unwrap();
    let layout = snapshot(&view, cx);
    assert!(layout.region(Region::Left).unwrap().open);
    assert_eq!(region_of(&layout, PanelKind::Library), Some(Region::Left));
}
