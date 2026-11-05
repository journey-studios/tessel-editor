//! Docking: the four workstation panels hosted by the Kit's `DockArea`.
//!
//! The `DockArea` is the only owner of the layout. Gestures (dragging a tab,
//! resizing a divider, the collapse buttons) edit it through the Kit, and the
//! CLI edits it through [`DockHost::apply`], so both end in the same live
//! tree. [`DockHost::snapshot`] reads that tree into the pure
//! [`Layout`](crate::state::layout::Layout) the CLI validates against; nothing
//! keeps a second copy.

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::dock::{
    BasePanel, BasePanelView, DockArea, DockAreaState, DockLayout, DockPlacement, DockSkin,
    InsertTarget, NodeId, PaneNode, PaneRef, Panel, PanelControl, PanelEvent, PanelId, PanelInfo,
    PanelState, panel_handle, register_panel,
};
use gpui_kit::component::h_flex;
use gpui_kit::component::{ActiveTheme, Icon, Placement, Sizable};
use gpui_kit::*;
use serde_json::{Value, json};

use super::Workstation;
use super::tokens::Tokens;
use crate::state::CommandError;
use crate::state::layout::{
    Axis, Destination, Layout, LayoutCommand, Node, PanelKind, Region, RegionState, Side,
};

pub const LEFT_SIZE: f32 = 232.0;
pub const RIGHT_SIZE: f32 = 272.0;
pub const BOTTOM_SIZE: f32 = 296.0;

/// Bump when the default layout or the panel names change shape, so a stored
/// layout from an older build is not loaded blindly.
const LAYOUT_VERSION: usize = 1;

fn default_size(region: Region) -> Option<f32> {
    match region {
        Region::Left => Some(LEFT_SIZE),
        Region::Right => Some(RIGHT_SIZE),
        Region::Bottom => Some(BOTTOM_SIZE),
        Region::Center => None,
    }
}

fn placement(region: Region) -> DockPlacement {
    match region {
        Region::Center => DockPlacement::Center,
        Region::Left => DockPlacement::Left,
        Region::Right => DockPlacement::Right,
        Region::Bottom => DockPlacement::Bottom,
    }
}

/// The name a panel is persisted under. Never change a released one.
pub fn persisted_name(kind: PanelKind) -> &'static str {
    match kind {
        PanelKind::Library => "tessel.library",
        PanelKind::Monitor => "tessel.monitor",
        PanelKind::Timeline => "tessel.timeline",
        PanelKind::Inspector => "tessel.inspector",
    }
}

fn kind_from_name(name: &str) -> Option<PanelKind> {
    PanelKind::ALL
        .into_iter()
        .find(|kind| persisted_name(*kind) == name)
}

fn title_of(kind: PanelKind) -> &'static str {
    match kind {
        PanelKind::Library => "Biblioteca de mídia",
        PanelKind::Monitor => "Monitor",
        PanelKind::Timeline => "Timeline",
        PanelKind::Inspector => "Inspetor",
    }
}

fn icon_of(kind: PanelKind) -> IconName {
    match kind {
        PanelKind::Library => IconName::Library,
        PanelKind::Monitor => IconName::Monitor,
        PanelKind::Timeline => IconName::ChartGantt,
        PanelKind::Inspector => IconName::SlidersHorizontal,
    }
}

/// One dockable region of the editor. It owns no state: it asks the
/// workstation to draw its region, so the panel can move between docks while
/// the editor state stays where it is.
pub struct WorkPanel {
    kind: PanelKind,
    workstation: WeakEntity<Workstation>,
    focus: FocusHandle,
}

impl WorkPanel {
    fn new(kind: PanelKind, workstation: WeakEntity<Workstation>, cx: &mut Context<Self>) -> Self {
        Self {
            kind,
            workstation,
            focus: cx.focus_handle(),
        }
    }
}

impl EventEmitter<PanelEvent> for WorkPanel {}

impl Focusable for WorkPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl BasePanel for WorkPanel {
    fn panel_name(&self) -> &'static str {
        persisted_name(self.kind)
    }

    // Zooming one panel to fill the dock would hide the workstation state the
    // CLI keeps reporting; the dock already offers resizing for that.
    fn zoomable(&self, _: &App) -> bool {
        false
    }
}

impl Panel for WorkPanel {
    fn tab_name(&self, _: &App) -> Option<SharedString> {
        Some(title_of(self.kind).into())
    }

    fn title(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let color = cx.theme().colors.muted_foreground;
        h_flex()
            .gap_1p5()
            .items_center()
            .child(Icon::new(icon_of(self.kind)).xsmall().text_color(color))
            .child(title_of(self.kind))
    }

    fn zoom_control(&self, _: &App) -> Option<PanelControl> {
        None
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl Render for WorkPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = self.workstation.upgrade().map(|workstation| {
            let tokens = Tokens::new(cx);
            let kind = self.kind;
            workstation.update(cx, |workstation, cx| {
                workstation.render_panel(kind, &tokens, window, cx)
            })
        });
        div()
            .id(SharedString::from(format!("panel-{}", self.kind.id())))
            .track_focus(&self.focus)
            .size_full()
            .min_w(px(0.))
            .min_h(px(0.))
            .children(content)
    }
}

pub struct DockHost {
    pub area: Entity<DockArea>,
    /// Held so the skin's presentation hooks live as long as the area.
    _skin: Rc<DockSkin>,
    panels: Vec<(PanelKind, Entity<WorkPanel>)>,
}

impl DockHost {
    /// Builds the four panels and the dock, installs the default layout and
    /// registers the panel builders `load` needs.
    pub fn new(workstation: WeakEntity<Workstation>, window: &mut Window, cx: &mut App) -> Self {
        let panels: Vec<(PanelKind, Entity<WorkPanel>)> = PanelKind::ALL
            .into_iter()
            .map(|kind| {
                let workstation = workstation.clone();
                (kind, cx.new(|cx| WorkPanel::new(kind, workstation, cx)))
            })
            .collect();
        let (area, skin) = DockSkin::dock_area("workstation", Some(LAYOUT_VERSION), window, cx);
        let host = Self {
            area,
            _skin: skin,
            panels,
        };
        host.register_builders(cx);
        host.reset(window, cx);
        host
    }

    fn entity(&self, kind: PanelKind) -> Entity<WorkPanel> {
        self.panels
            .iter()
            .find(|(candidate, _)| *candidate == kind)
            .map(|(_, entity)| entity.clone())
            .expect("every panel kind has an entity")
    }

    fn handle(&self, kind: PanelKind) -> Arc<dyn BasePanelView> {
        panel_handle(self.entity(kind))
    }

    /// The panel views, so the workstation can repaint them together.
    pub fn views(&self) -> Vec<Entity<WorkPanel>> {
        self.panels
            .iter()
            .map(|(_, entity)| entity.clone())
            .collect()
    }

    fn kind_of(&self, id: PanelId) -> Option<PanelKind> {
        self.panels
            .iter()
            .find(|(_, entity)| PanelId::from(entity.entity_id()) == id)
            .map(|(kind, _)| *kind)
    }

    fn register_builders(&self, cx: &mut App) {
        for (kind, entity) in &self.panels {
            let entity = entity.clone();
            register_panel(cx, persisted_name(*kind), move |_, _, _| {
                panel_handle(entity.clone())
            });
        }
    }

    /// The default layout: library left, monitor centre, timeline below,
    /// inspector right.
    pub fn reset(&self, window: &mut Window, cx: &mut App) {
        self.area.update(cx, |area, cx| {
            area.set_center(
                DockLayout::h_split().child(
                    DockLayout::tabs().panel_view(self.handle(PanelKind::Monitor), cx),
                    None,
                ),
                window,
                cx,
            );
            for kind in [
                PanelKind::Library,
                PanelKind::Inspector,
                PanelKind::Timeline,
            ] {
                let region = kind.home();
                area.set_dock(
                    placement(region),
                    DockLayout::tabs().panel_view(self.handle(kind), cx),
                    window,
                    cx,
                );
                if let Some(size) = default_size(region) {
                    area.set_dock_size(placement(region), px(size), window, cx);
                }
                area.set_dock_collapsible(placement(region), true, window, cx);
                if !area.is_dock_open(placement(region)) {
                    area.toggle_dock(placement(region), window, cx);
                }
            }
        });
    }

    // ---- reading the live tree ----

    pub fn snapshot(&self, cx: &App) -> Layout {
        let area = self.area.read(cx);
        let dump = area.dump(cx);
        let bounds = area.bounds();
        let regions = Region::ALL
            .into_iter()
            .map(|region| {
                let dock = placement(region);
                let state = match region {
                    Region::Center => Some(&dump.center),
                    Region::Left => dump.left_dock.as_ref().map(|dock| dock.panel()),
                    Region::Right => dump.right_dock.as_ref().map(|dock| dock.panel()),
                    Region::Bottom => dump.bottom_dock.as_ref().map(|dock| dock.panel()),
                };
                let root = match (area.layout(dock), state) {
                    (Some(tree), Some(state)) => self.convert(tree.root(), state),
                    _ => Node::Split {
                        id: 0,
                        axis: Axis::Horizontal,
                        sizes: Vec::new(),
                        children: Vec::new(),
                    },
                };
                RegionState {
                    region,
                    open: !region.is_dock() || area.is_dock_open(dock),
                    size: region
                        .is_dock()
                        .then(|| {
                            area.dock_size(dock)
                                .map(f32::from)
                                .or_else(|| default_size(region))
                        })
                        .flatten(),
                    collapsible: area.is_dock_collapsible(dock),
                    root,
                }
            })
            .collect();
        Layout {
            regions,
            area: (f32::from(bounds.size.width), f32::from(bounds.size.height)),
        }
    }

    /// Walks the live tree and the dump in step: the tree carries the node IDs,
    /// the dump carries the measured slot sizes.
    fn convert(&self, node: &PaneNode, state: &PanelState) -> Node {
        match node.kind() {
            PaneRef::Split {
                axis,
                children,
                sizes,
            } => {
                let measured = state.info.sizes();
                Node::Split {
                    id: node.id().as_u64(),
                    axis: if axis == gpui_kit::Axis::Horizontal {
                        Axis::Horizontal
                    } else {
                        Axis::Vertical
                    },
                    sizes: (0..children.len())
                        .map(|index| {
                            measured
                                .and_then(|sizes| sizes.get(index).copied())
                                .or_else(|| sizes.get(index).copied().flatten())
                                .map_or(0.0, f32::from)
                        })
                        .collect(),
                    children: children
                        .iter()
                        .zip(&state.children)
                        .map(|(child, child_state)| self.convert(child, child_state))
                        .collect(),
                }
            }
            PaneRef::Tabs { panels, active_ix } => Node::Group {
                id: node.id().as_u64(),
                active: active_ix,
                panels: panels.iter().filter_map(|id| self.kind_of(*id)).collect(),
            },
        }
    }

    fn node(&self, raw: u64, cx: &App) -> Option<NodeId> {
        let area = self.area.read(cx);
        Region::ALL.into_iter().find_map(|region| {
            area.layout(placement(region))?
                .node_ids()
                .into_iter()
                .find(|id| id.as_u64() == raw)
        })
    }

    // ---- applying a validated command ----

    /// Applies `command` to the live area. The caller has already validated it
    /// against a snapshot, so the Kit calls below cannot meet an input that
    /// would make them silently do nothing.
    pub fn apply(
        &self,
        command: &LayoutCommand,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<Value, CommandError> {
        match command {
            LayoutCommand::SetVisible { panel, visible } => {
                if *visible {
                    self.reveal(*panel, window, cx);
                } else {
                    self.area.update(cx, |area, cx| {
                        area.remove_panel(self.entity(*panel), window, cx)
                    });
                    self.close_empty_docks(window, cx);
                }
                Ok(json!({"target": format!("panel:{}", panel.id()), "visible": visible}))
            }
            LayoutCommand::Activate { panel } => {
                self.reveal(*panel, window, cx);
                Ok(json!({"target": format!("panel:{}", panel.id()), "action": "activate"}))
            }
            LayoutCommand::Move { panel, to } => {
                self.move_to(*panel, to, window, cx)?;
                self.close_empty_docks(window, cx);
                Ok(json!({"target": format!("panel:{}", panel.id()), "action": "move"}))
            }
            LayoutCommand::ResizeRegion { region, size } => {
                self.area.update(cx, |area, cx| {
                    area.set_dock_size(placement(*region), px(*size), window, cx)
                });
                Ok(json!({"target": format!("dock:{}", region.id()), "size": size}))
            }
            LayoutCommand::SetRegionOpen { region, open } => {
                self.set_open(*region, *open, window, cx);
                Ok(json!({"target": format!("dock:{}", region.id()), "open": open}))
            }
            LayoutCommand::ResizeSplit { split, sizes } => {
                let node = self.node(*split, cx).ok_or_else(|| {
                    CommandError::new("unknown_element", format!("No split `split:{split}`"))
                })?;
                self.area.update(cx, |area, cx| {
                    area.set_split_sizes(
                        node,
                        sizes.iter().map(|size| px(*size)).collect(),
                        window,
                        cx,
                    )
                });
                Ok(json!({"target": format!("split:{split}"), "sizes": sizes}))
            }
            LayoutCommand::Reset => {
                self.reset(window, cx);
                Ok(json!({"target": "dock", "action": "reset"}))
            }
            LayoutCommand::Load { json } => {
                self.load(json, window, cx)?;
                Ok(json!({"target": "dock", "action": "load"}))
            }
        }
    }

    /// Collapses side docks that no longer hold a panel. The Kit keeps an empty
    /// dock open at its old width, which would leave a blank strip beside the
    /// monitor; `place`/`move_to`/`reveal` reopen a dock when a panel returns.
    pub fn close_empty_docks(&self, window: &mut Window, cx: &mut App) {
        for region in Region::ALL.into_iter().filter(|region| region.is_dock()) {
            let empty = self.area.read(cx).is_empty(placement(region), cx);
            if empty {
                self.set_open(region, false, window, cx);
            }
        }
    }

    fn set_open(&self, region: Region, open: bool, window: &mut Window, cx: &mut App) {
        self.area.update(cx, |area, cx| {
            if area.is_dock_open(placement(region)) != open {
                area.toggle_dock(placement(region), window, cx);
            }
        });
    }

    /// Puts the panel on screen: placed in its home dock if it was hidden, its
    /// dock opened if it was collapsed, and its tab brought to the front.
    fn reveal(&self, kind: PanelKind, window: &mut Window, cx: &mut App) {
        let placed = self.snapshot(cx).locate(kind);
        let region = placed.map_or(kind.home(), |found| found.region);
        if placed.is_none() {
            self.place(kind, region, window, cx);
        }
        if region.is_dock() {
            self.set_open(region, true, window, cx);
        }
        self.area.update(cx, |area, cx| {
            area.select_panel(PanelId::from(self.entity(kind).entity_id()), window, cx)
        });
    }

    fn place(&self, kind: PanelKind, region: Region, window: &mut Window, cx: &mut App) {
        let size = default_size(region).map(px);
        self.area.update(cx, |area, cx| {
            area.add_panel_view(self.handle(kind), placement(region), size, window, cx)
        });
    }

    fn move_to(
        &self,
        kind: PanelKind,
        to: &Destination,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<(), CommandError> {
        let id = PanelId::from(self.entity(kind).entity_id());
        let layout = self.snapshot(cx);
        let was_placed = layout.locate(kind).is_some();
        let region_of_group = |group: u64| layout.group(group).map(|(region, _)| region);
        let target = match to {
            Destination::Region(region) => {
                // The Kit places a hidden panel itself; a placed one moves into
                // the region's first tab group.
                if !was_placed {
                    self.place(kind, *region, window, cx);
                    self.set_open(*region, true, window, cx);
                    return Ok(());
                }
                let first_group = layout
                    .region(*region)
                    .and_then(|state| first_group(&state.root));
                let Some(group) = first_group.and_then(|group| self.node(group, cx)) else {
                    self.area.update(cx, |area, cx| {
                        area.remove_panel(self.entity(kind), window, cx)
                    });
                    self.place(kind, *region, window, cx);
                    self.set_open(*region, true, window, cx);
                    return Ok(());
                };
                (
                    *region,
                    InsertTarget::Tabs {
                        node: group,
                        ix: None,
                        activate: true,
                    },
                )
            }
            Destination::Group { group, index } => {
                let node = self.node(*group, cx).ok_or_else(|| missing_group(*group))?;
                let region = region_of_group(*group).ok_or_else(|| missing_group(*group))?;
                (
                    region,
                    InsertTarget::Tabs {
                        node,
                        ix: *index,
                        activate: true,
                    },
                )
            }
            Destination::Beside { group, side } => {
                let node = self.node(*group, cx).ok_or_else(|| missing_group(*group))?;
                let region = region_of_group(*group).ok_or_else(|| missing_group(*group))?;
                (
                    region,
                    InsertTarget::Split {
                        node,
                        placement: match side {
                            Side::Left => Placement::Left,
                            Side::Right => Placement::Right,
                            Side::Top => Placement::Top,
                            Side::Bottom => Placement::Bottom,
                        },
                        size: None,
                    },
                )
            }
        };
        let (region, insert) = target;
        if !was_placed {
            // `move_panel` only moves what the area already owns.
            self.place(kind, region, window, cx);
        }
        self.area
            .update(cx, |area, cx| area.move_panel(id, insert, window, cx));
        if region.is_dock() {
            self.set_open(region, true, window, cx);
        }
        Ok(())
    }

    // ---- persistence ----

    /// The Kit's own dump of the live layout.
    pub fn dump(&self, cx: &App) -> Value {
        serde_json::to_value(self.area.read(cx).dump(cx)).unwrap_or(Value::Null)
    }

    /// Rejects anything that is not a layout of exactly these four panels
    /// before the area is touched, so a bad file cannot strand the editor
    /// with placeholder panels.
    fn parse_state(json: &str) -> Result<DockAreaState, CommandError> {
        let invalid = |message: String| CommandError::new("invalid_layout", message);
        let state: DockAreaState = serde_json::from_str(json)
            .map_err(|error| invalid(format!("Not a dock layout: {error}")))?;
        let mut names = Vec::new();
        collect_names(&state.center, &mut names);
        for dock in [&state.left_dock, &state.right_dock, &state.bottom_dock]
            .into_iter()
            .flatten()
        {
            collect_names(dock.panel(), &mut names);
        }
        if let Some(unknown) = names.iter().find(|name| kind_from_name(name).is_none()) {
            return Err(invalid(format!("Unknown panel `{unknown}`")));
        }
        let mut seen = names.clone();
        seen.sort_unstable();
        seen.dedup();
        if seen.len() != names.len() {
            return Err(invalid("A panel appears more than once".into()));
        }
        Ok(state)
    }

    fn load(&self, json: &str, window: &mut Window, cx: &mut App) -> Result<(), CommandError> {
        let state = Self::parse_state(json)?;
        let result = self
            .area
            .update(cx, |area, cx| area.load(state, window, cx));
        result.map_err(|error| CommandError::new("invalid_layout", error.to_string()))
    }
}

fn collect_names(state: &PanelState, names: &mut Vec<String>) {
    // Containers (splits, tab groups) carry their own names; only leaves are panels.
    if matches!(state.info, PanelInfo::Panel(_)) {
        names.push(state.panel_name.clone());
    }
    for child in &state.children {
        collect_names(child, names);
    }
}

fn missing_group(group: u64) -> CommandError {
    CommandError::new("unknown_element", format!("No group `group:{group}`"))
        .target(format!("group:{group}"))
}

fn first_group(node: &Node) -> Option<u64> {
    match node {
        Node::Group { id, .. } => Some(*id),
        Node::Split { children, .. } => children.iter().find_map(first_group),
    }
}
