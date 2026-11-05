//! The dock layout as plain data, plus the validation of layout commands.
//!
//! The live `DockArea` owns the real layout. This module never stores one: the
//! workstation snapshots the area into a [`Layout`] whenever the CLI asks a
//! question or sends a command, so there is no second tree that could drift.
//! Every command is validated against that snapshot *before* the area is
//! touched, which is what makes a rejected command leave the layout intact.

use serde_json::{Value, json};

use super::error::CommandError;

/// Smallest extent of a dock or a split slot. Mirrors the Kit's `PANEL_MIN_SIZE`
/// (100 px) so the CLI never asks for a size the dock would silently clamp.
pub const MIN_PANE: f32 = 100.0;

/// A dockable panel. The set is closed: these are the four regions of the
/// editor, and the CLI addresses them as `panel:<id>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PanelKind {
    Library,
    Monitor,
    Timeline,
    Inspector,
}

impl PanelKind {
    pub const ALL: [Self; 4] = [
        Self::Library,
        Self::Monitor,
        Self::Timeline,
        Self::Inspector,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Library => "library",
            Self::Monitor => "monitor",
            Self::Timeline => "timeline",
            Self::Inspector => "inspector",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.id() == raw.trim())
    }

    /// Where the default layout places the panel, and where `show` returns it
    /// the first time.
    pub fn home(self) -> Region {
        match self {
            Self::Library => Region::Left,
            Self::Monitor => Region::Center,
            Self::Timeline => Region::Bottom,
            Self::Inspector => Region::Right,
        }
    }
}

/// One of the four areas of the `DockArea`: the center and three docks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Region {
    Center,
    Left,
    Right,
    Bottom,
}

impl Region {
    pub const ALL: [Self; 4] = [Self::Center, Self::Left, Self::Right, Self::Bottom];

    pub fn id(self) -> &'static str {
        match self {
            Self::Center => "center",
            Self::Left => "left",
            Self::Right => "right",
            Self::Bottom => "bottom",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|region| region.id() == raw.trim())
    }

    pub fn is_dock(self) -> bool {
        self != Self::Center
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

impl Axis {
    pub fn id(self) -> &'static str {
        match self {
            Self::Horizontal => "horizontal",
            Self::Vertical => "vertical",
        }
    }
}

/// Which edge of a group a panel is split onto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    pub const ALL: [Self; 4] = [Self::Left, Self::Right, Self::Top, Self::Bottom];

    pub fn id(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Top => "top",
            Self::Bottom => "bottom",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|side| side.id() == raw.trim())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Split {
        id: u64,
        axis: Axis,
        /// Measured pixel extent of each slot, in child order.
        sizes: Vec<f32>,
        children: Vec<Node>,
    },
    Group {
        id: u64,
        active: usize,
        panels: Vec<PanelKind>,
    },
}

impl Node {
    pub fn dom_id(&self) -> String {
        match self {
            Self::Split { id, .. } => format!("split:{id}"),
            Self::Group { id, .. } => format!("group:{id}"),
        }
    }

    fn walk<'a>(&'a self, visit: &mut impl FnMut(&'a Node)) {
        visit(self);
        if let Self::Split { children, .. } = self {
            for child in children {
                child.walk(visit);
            }
        }
    }

    pub fn to_json(&self) -> Value {
        match self {
            Self::Split {
                axis,
                sizes,
                children,
                ..
            } => json!({
                "id": self.dom_id(), "kind": "split", "axis": axis.id(),
                "sizes": sizes, "children": children.iter().map(Node::to_json).collect::<Vec<_>>(),
            }),
            Self::Group { active, panels, .. } => json!({
                "id": self.dom_id(), "kind": "group", "active": active,
                "panels": panels.iter().map(|panel| format!("panel:{}", panel.id())).collect::<Vec<_>>(),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RegionState {
    pub region: Region,
    /// Docks can be collapsed; the center is always open.
    pub open: bool,
    /// Extent of the dock along its axis; `None` for the center.
    pub size: Option<f32>,
    pub collapsible: bool,
    pub root: Node,
}

/// Where a panel currently sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub region: Region,
    pub group: u64,
    pub index: usize,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub regions: Vec<RegionState>,
    /// Width and height of the dock area, 0 until it has been painted.
    pub area: (f32, f32),
}

/// Where a moved or revealed panel lands.
#[derive(Debug, Clone, PartialEq)]
pub enum Destination {
    /// Into the region's first tab group, or a new group when it has none.
    Region(Region),
    /// As a tab of a group, at `index` or at the end.
    Group { group: u64, index: Option<usize> },
    /// In a new group beside an existing one.
    Beside { group: u64, side: Side },
}

/// A group named either by its numeric ID or by a panel it currently holds, so
/// scripts do not depend on IDs that only exist while the group does.
#[derive(Debug, Clone, PartialEq)]
pub enum GroupRef {
    Id(u64),
    HoldingPanel(PanelKind),
}

#[derive(Debug, Clone, PartialEq)]
pub enum LayoutCommand {
    SetVisible {
        panel: PanelKind,
        visible: bool,
    },
    Activate {
        panel: PanelKind,
    },
    Move {
        panel: PanelKind,
        to: Destination,
    },
    ResizeRegion {
        region: Region,
        size: f32,
    },
    SetRegionOpen {
        region: Region,
        open: bool,
    },
    ResizeSplit {
        split: u64,
        sizes: Vec<f32>,
    },
    Reset,
    /// The JSON text of a dump produced by `dock dump`.
    Load {
        json: String,
    },
}

fn unknown(kind: &str, id: &str, available: Vec<String>) -> CommandError {
    CommandError::new("unknown_element", format!("No {kind} with ID `{id}`"))
        .target(id)
        .details(json!({ "kind": kind, "available": available }))
}

impl Layout {
    pub fn region(&self, region: Region) -> Option<&RegionState> {
        self.regions.iter().find(|state| state.region == region)
    }

    fn nodes(&self) -> Vec<(Region, &Node)> {
        let mut found = Vec::new();
        for state in &self.regions {
            state
                .root
                .walk(&mut |node| found.push((state.region, node)));
        }
        found
    }

    pub fn group_ids(&self) -> Vec<u64> {
        self.nodes()
            .into_iter()
            .filter_map(|(_, node)| match node {
                Node::Group { id, .. } => Some(*id),
                Node::Split { .. } => None,
            })
            .collect()
    }

    pub fn split_ids(&self) -> Vec<u64> {
        self.nodes()
            .into_iter()
            .filter_map(|(_, node)| match node {
                Node::Split { id, .. } => Some(*id),
                Node::Group { .. } => None,
            })
            .collect()
    }

    pub fn group(&self, id: u64) -> Option<(Region, &Node)> {
        self.nodes()
            .into_iter()
            .find(|(_, node)| matches!(node, Node::Group { id: group, .. } if *group == id))
    }

    pub fn split(&self, id: u64) -> Option<(Region, &Node)> {
        self.nodes()
            .into_iter()
            .find(|(_, node)| matches!(node, Node::Split { id: split, .. } if *split == id))
    }

    pub fn locate(&self, panel: PanelKind) -> Option<Placement> {
        for (region, node) in self.nodes() {
            let Node::Group { id, active, panels } = node else {
                continue;
            };
            if let Some(index) = panels.iter().position(|held| *held == panel) {
                return Some(Placement {
                    region,
                    group: *id,
                    index,
                    active: *active == index,
                });
            }
        }
        None
    }

    /// Panels not placed anywhere: closed, waiting for `show`.
    pub fn hidden(&self) -> Vec<PanelKind> {
        PanelKind::ALL
            .into_iter()
            .filter(|panel| self.locate(*panel).is_none())
            .collect()
    }

    /// Whether the panel is both placed and on screen (its dock is open).
    pub fn is_visible(&self, panel: PanelKind) -> bool {
        self.locate(panel)
            .and_then(|placement| self.region(placement.region))
            .is_some_and(|state| state.open)
    }

    pub fn resolve_group(&self, group: &GroupRef) -> Result<u64, CommandError> {
        match group {
            GroupRef::Id(id) => self
                .group(*id)
                .map(|_| *id)
                .ok_or_else(|| self.unknown_group(&format!("group:{id}"))),
            GroupRef::HoldingPanel(panel) => self
                .locate(*panel)
                .map(|placement| placement.group)
                .ok_or_else(|| {
                    CommandError::new(
                        "not_applicable",
                        format!("`panel:{}` is hidden, so it holds no group", panel.id()),
                    )
                    .target(format!("panel:{}", panel.id()))
                }),
        }
    }

    fn unknown_group(&self, id: &str) -> CommandError {
        unknown(
            "group",
            id,
            self.group_ids()
                .into_iter()
                .map(|id| format!("group:{id}"))
                .collect(),
        )
    }

    pub fn to_json(&self) -> Value {
        let panels: Vec<Value> = PanelKind::ALL
            .into_iter()
            .map(|panel| {
                let placement = self.locate(panel);
                json!({
                    "id": format!("panel:{}", panel.id()),
                    "panel": panel.id(),
                    "visible": self.is_visible(panel),
                    "placed": placement.is_some(),
                    "region": placement.map(|found| found.region.id()),
                    "group": placement.map(|found| format!("group:{}", found.group)),
                    "index": placement.map(|found| found.index),
                    "active": placement.is_some_and(|found| found.active),
                })
            })
            .collect();
        json!({
            "area": { "width": self.area.0, "height": self.area.1 },
            "regions": self.regions.iter().map(|state| json!({
                "id": format!("dock:{}", state.region.id()),
                "region": state.region.id(),
                "open": state.open,
                "size": state.size,
                "collapsible": state.collapsible,
                "root": state.root.to_json(),
            })).collect::<Vec<_>>(),
            "panels": panels,
            "hidden": self.hidden().into_iter().map(PanelKind::id).collect::<Vec<_>>(),
        })
    }

    /// Largest extent a dock may take: the area minus the minimum the rest
    /// needs and the opposite dock. Unknown until the area has been painted.
    pub fn max_dock_size(&self, region: Region) -> Option<f32> {
        let (width, height) = self.area;
        let opposite = |other: Region| {
            self.region(other)
                .filter(|state| state.open)
                .and_then(|state| state.size)
                .unwrap_or(0.0)
        };
        let max = match region {
            Region::Left => width - MIN_PANE - opposite(Region::Right),
            Region::Right => width - MIN_PANE - opposite(Region::Left),
            Region::Bottom => height - MIN_PANE,
            Region::Center => return None,
        };
        (max > 0.0).then_some(max.max(MIN_PANE))
    }

    /// Rejects a command that cannot apply, naming why. Pure: the area is only
    /// modified after this returns `Ok`.
    pub fn validate(&self, command: &LayoutCommand) -> Result<(), CommandError> {
        match command {
            LayoutCommand::SetVisible { .. } | LayoutCommand::Reset => Ok(()),
            LayoutCommand::Load { json } => serde_json::from_str::<Value>(json)
                .map(|_| ())
                .map_err(|error| {
                    CommandError::new("invalid_layout", format!("Layout is not JSON: {error}"))
                }),
            LayoutCommand::Activate { panel } => self.validate_activate(*panel),
            LayoutCommand::Move { panel, to } => self.validate_move(*panel, to),
            LayoutCommand::ResizeRegion { region, size } => self.validate_resize(*region, *size),
            LayoutCommand::SetRegionOpen { region, open } => {
                let state = self.dock_state(*region)?;
                if !open && !state.collapsible {
                    return Err(CommandError::new(
                        "not_applicable",
                        format!("`dock:{}` cannot be collapsed", region.id()),
                    )
                    .target(format!("dock:{}", region.id())));
                }
                Ok(())
            }
            LayoutCommand::ResizeSplit { split, sizes } => self.validate_split(*split, sizes),
        }
    }

    fn dock_state(&self, region: Region) -> Result<&RegionState, CommandError> {
        let target = format!("dock:{}", region.id());
        if !region.is_dock() {
            return Err(CommandError::new(
                "not_applicable",
                "The center takes whatever the docks leave; resize a dock or a split instead",
            )
            .target(target));
        }
        self.region(region).ok_or_else(|| {
            unknown(
                "dock",
                &target,
                self.regions
                    .iter()
                    .map(|state| format!("dock:{}", state.region.id()))
                    .collect(),
            )
        })
    }

    fn validate_activate(&self, panel: PanelKind) -> Result<(), CommandError> {
        if self.locate(panel).is_none() {
            return Err(CommandError::new(
                "not_applicable",
                format!("`panel:{}` is hidden; show it first", panel.id()),
            )
            .target(format!("panel:{}", panel.id())));
        }
        Ok(())
    }

    fn validate_move(&self, panel: PanelKind, to: &Destination) -> Result<(), CommandError> {
        let target = format!("panel:{}", panel.id());
        let current = self.locate(panel);
        match to {
            Destination::Region(region) => self
                .region(*region)
                .map(|_| ())
                .ok_or_else(|| unknown("dock", &format!("dock:{}", region.id()), vec![])),
            Destination::Group { group, index } => {
                let (_, node) = self
                    .group(*group)
                    .ok_or_else(|| self.unknown_group(&format!("group:{group}")))?;
                let Node::Group { panels, .. } = node else {
                    return Ok(());
                };
                let already_there = current.is_some_and(|found| found.group == *group);
                // After the panel leaves its own group the usable slots shrink by one.
                let slots = panels.len() - usize::from(already_there);
                match index {
                    Some(index) if *index > slots => Err(CommandError::new(
                        "out_of_bounds",
                        format!("Index {index} is outside the group; use 0 to {slots}"),
                    )
                    .target(target)
                    .details(json!({ "min": 0, "max": slots }))),
                    _ => Ok(()),
                }
            }
            Destination::Beside { group, .. } => {
                let (_, node) = self
                    .group(*group)
                    .ok_or_else(|| self.unknown_group(&format!("group:{group}")))?;
                let Node::Group { panels, .. } = node else {
                    return Ok(());
                };
                // Moving the only occupant out dissolves the group it should
                // be placed beside, leaving nothing to anchor the split to.
                if panels == &[panel] {
                    return Err(CommandError::new(
                        "invalid_target",
                        format!(
                            "`group:{group}` only holds `panel:{}`; it cannot be split beside itself",
                            panel.id()
                        ),
                    )
                    .target(target));
                }
                Ok(())
            }
        }
    }

    fn validate_resize(&self, region: Region, size: f32) -> Result<(), CommandError> {
        self.dock_state(region)?;
        let target = format!("dock:{}", region.id());
        let max = self.max_dock_size(region);
        if !size.is_finite() || size < MIN_PANE || max.is_some_and(|max| size > max) {
            return Err(CommandError::new(
                "out_of_bounds",
                format!(
                    "Size must be between {MIN_PANE} and {}",
                    max.map_or("the window".to_string(), |max| max.to_string())
                ),
            )
            .target(target)
            .details(json!({ "min": MIN_PANE, "max": max })));
        }
        Ok(())
    }

    fn validate_split(&self, split: u64, sizes: &[f32]) -> Result<(), CommandError> {
        let target = format!("split:{split}");
        let Some((
            _,
            Node::Split {
                sizes: current,
                children,
                ..
            },
        )) = self.split(split)
        else {
            return Err(unknown(
                "split",
                &target,
                self.split_ids()
                    .into_iter()
                    .map(|id| format!("split:{id}"))
                    .collect(),
            ));
        };
        if sizes.len() != children.len() {
            return Err(CommandError::invalid_value(
                format!("Expected {} sizes, one per child", children.len()),
                json!({ "count": children.len() }),
            )
            .target(target));
        }
        if let Some(bad) = sizes
            .iter()
            .find(|size| !size.is_finite() || **size < MIN_PANE)
        {
            return Err(CommandError::new(
                "out_of_bounds",
                format!("Every slot needs at least {MIN_PANE} px (got {bad})"),
            )
            .target(target)
            .details(json!({ "min": MIN_PANE })));
        }
        let total: f32 = current.iter().sum();
        let requested: f32 = sizes.iter().sum();
        // The slots always fill their container, so a different total cannot be honoured.
        if total > 0.0 && (total - requested).abs() > 1.0 {
            return Err(CommandError::invalid_value(
                format!("Sizes must add up to the split's {total} px (got {requested})"),
                json!({ "total": total }),
            )
            .target(target));
        }
        Ok(())
    }
}

/// Builds a layout for tests and documentation: one group per entry.
#[cfg(test)]
pub fn default_layout() -> Layout {
    let group = |id, panel| Node::Group {
        id,
        active: 0,
        panels: vec![panel],
    };
    Layout {
        area: (1440.0, 840.0),
        regions: vec![
            RegionState {
                region: Region::Center,
                open: true,
                size: None,
                collapsible: false,
                root: Node::Split {
                    id: 1,
                    axis: Axis::Horizontal,
                    sizes: vec![936.0],
                    children: vec![group(2, PanelKind::Monitor)],
                },
            },
            RegionState {
                region: Region::Left,
                open: true,
                size: Some(232.0),
                collapsible: true,
                root: Node::Split {
                    id: 3,
                    axis: Axis::Horizontal,
                    sizes: vec![232.0],
                    children: vec![group(4, PanelKind::Library)],
                },
            },
            RegionState {
                region: Region::Right,
                open: true,
                size: Some(272.0),
                collapsible: true,
                root: Node::Split {
                    id: 5,
                    axis: Axis::Horizontal,
                    sizes: vec![272.0],
                    children: vec![group(6, PanelKind::Inspector)],
                },
            },
            RegionState {
                region: Region::Bottom,
                open: true,
                size: Some(296.0),
                collapsible: true,
                root: Node::Split {
                    id: 7,
                    axis: Axis::Horizontal,
                    sizes: vec![936.0],
                    children: vec![group(8, PanelKind::Timeline)],
                },
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_visibility_is_a_layout_fact() {
        // Ported from `workspace_tabs_and_panels_hold_state`: hiding a panel
        // is now a fact about the dock, not a flag in the editor state.
        let mut layout = default_layout();
        assert!(layout.is_visible(PanelKind::Library));
        assert_eq!(PanelKind::parse("timeline"), Some(PanelKind::Timeline));
        assert_eq!(PanelKind::parse("status"), None);
        layout.regions[1].root = Node::Split {
            id: 3,
            axis: Axis::Horizontal,
            sizes: vec![],
            children: vec![],
        };
        assert!(!layout.is_visible(PanelKind::Library));
        assert_eq!(layout.hidden(), vec![PanelKind::Library]);
    }

    #[test]
    fn closed_docks_do_not_show_their_panels() {
        let mut layout = default_layout();
        layout.regions[3].open = false;
        assert!(!layout.is_visible(PanelKind::Timeline));
        assert!(layout.locate(PanelKind::Timeline).is_some());
    }

    #[test]
    fn panels_are_located_with_group_and_index() {
        let layout = default_layout();
        let found = layout.locate(PanelKind::Inspector).unwrap();
        assert_eq!(found.region, Region::Right);
        assert_eq!(found.group, 6);
        assert_eq!(found.index, 0);
        assert!(found.active);
        assert_eq!(layout.group_ids(), vec![2, 4, 6, 8]);
        assert_eq!(layout.split_ids(), vec![1, 3, 5, 7]);
    }

    #[test]
    fn unknown_targets_list_what_exists() {
        let layout = default_layout();
        let error = layout
            .validate(&LayoutCommand::Move {
                panel: PanelKind::Library,
                to: Destination::Group {
                    group: 99,
                    index: None,
                },
            })
            .unwrap_err();
        assert_eq!(error.code, "unknown_element");
        assert_eq!(error.target.as_deref(), Some("group:99"));
        assert_eq!(
            error.details.as_ref().unwrap()["available"],
            json!(["group:2", "group:4", "group:6", "group:8"])
        );
    }

    #[test]
    fn tab_index_must_fit_the_group() {
        let layout = default_layout();
        let into = |index| LayoutCommand::Move {
            panel: PanelKind::Library,
            to: Destination::Group {
                group: 2,
                index: Some(index),
            },
        };
        assert!(layout.validate(&into(1)).is_ok());
        assert_eq!(layout.validate(&into(2)).unwrap_err().code, "out_of_bounds");
        // Within its own single-panel group only slot 0 exists.
        let own = LayoutCommand::Move {
            panel: PanelKind::Library,
            to: Destination::Group {
                group: 4,
                index: Some(1),
            },
        };
        assert_eq!(layout.validate(&own).unwrap_err().code, "out_of_bounds");
    }

    #[test]
    fn a_lone_panel_cannot_be_split_beside_its_own_group() {
        let layout = default_layout();
        let error = layout
            .validate(&LayoutCommand::Move {
                panel: PanelKind::Library,
                to: Destination::Beside {
                    group: 4,
                    side: Side::Right,
                },
            })
            .unwrap_err();
        assert_eq!(error.code, "invalid_target");
        assert!(
            layout
                .validate(&LayoutCommand::Move {
                    panel: PanelKind::Library,
                    to: Destination::Beside {
                        group: 2,
                        side: Side::Right,
                    },
                })
                .is_ok()
        );
    }

    #[test]
    fn dock_sizes_are_bounded_by_the_area_and_the_opposite_dock() {
        let layout = default_layout();
        let resize = |region, size| LayoutCommand::ResizeRegion { region, size };
        assert!(layout.validate(&resize(Region::Left, 300.0)).is_ok());
        // 1440 - 100 - 272 (the open right dock) = 1068.
        assert!(layout.validate(&resize(Region::Left, 1068.0)).is_ok());
        for bad in [99.0, 1069.0, f32::NAN, f32::INFINITY] {
            assert_eq!(
                layout
                    .validate(&resize(Region::Left, bad))
                    .unwrap_err()
                    .code,
                "out_of_bounds",
                "{bad}"
            );
        }
        assert_eq!(
            layout
                .validate(&resize(Region::Center, 300.0))
                .unwrap_err()
                .code,
            "not_applicable"
        );
        assert_eq!(layout.max_dock_size(Region::Bottom), Some(740.0));
    }

    #[test]
    fn split_resizing_checks_count_minimum_and_total() {
        let mut layout = default_layout();
        layout.regions[0].root = Node::Split {
            id: 1,
            axis: Axis::Horizontal,
            sizes: vec![500.0, 436.0],
            children: vec![
                Node::Group {
                    id: 2,
                    active: 0,
                    panels: vec![PanelKind::Monitor],
                },
                Node::Group {
                    id: 9,
                    active: 0,
                    panels: vec![PanelKind::Inspector],
                },
            ],
        };
        let resize = |sizes: Vec<f32>| LayoutCommand::ResizeSplit { split: 1, sizes };
        assert!(layout.validate(&resize(vec![300.0, 636.0])).is_ok());
        assert_eq!(
            layout.validate(&resize(vec![936.0])).unwrap_err().code,
            "invalid_value"
        );
        assert_eq!(
            layout
                .validate(&resize(vec![50.0, 886.0]))
                .unwrap_err()
                .code,
            "out_of_bounds"
        );
        assert_eq!(
            layout
                .validate(&resize(vec![300.0, 300.0]))
                .unwrap_err()
                .code,
            "invalid_value"
        );
        assert_eq!(
            layout
                .validate(&LayoutCommand::ResizeSplit {
                    split: 77,
                    sizes: vec![]
                })
                .unwrap_err()
                .code,
            "unknown_element"
        );
    }

    #[test]
    fn hidden_panels_cannot_be_activated_and_groups_resolve_by_panel() {
        let mut layout = default_layout();
        layout.regions[3].root = Node::Split {
            id: 7,
            axis: Axis::Horizontal,
            sizes: vec![],
            children: vec![],
        };
        assert_eq!(
            layout
                .validate(&LayoutCommand::Activate {
                    panel: PanelKind::Timeline
                })
                .unwrap_err()
                .code,
            "not_applicable"
        );
        assert_eq!(
            layout.resolve_group(&GroupRef::HoldingPanel(PanelKind::Monitor)),
            Ok(2)
        );
        assert_eq!(
            layout
                .resolve_group(&GroupRef::HoldingPanel(PanelKind::Timeline))
                .unwrap_err()
                .code,
            "not_applicable"
        );
        assert_eq!(layout.resolve_group(&GroupRef::Id(4)), Ok(4));
        assert_eq!(
            layout.resolve_group(&GroupRef::Id(5)).unwrap_err().code,
            "unknown_element"
        );
    }

    #[test]
    fn the_center_cannot_collapse_and_json_lists_every_panel() {
        let layout = default_layout();
        assert_eq!(
            layout
                .validate(&LayoutCommand::SetRegionOpen {
                    region: Region::Center,
                    open: false
                })
                .unwrap_err()
                .code,
            "not_applicable"
        );
        let json = layout.to_json();
        assert_eq!(json["panels"].as_array().unwrap().len(), 4);
        assert_eq!(json["regions"][1]["id"], "dock:left");
        assert_eq!(json["hidden"], json!([]));
        assert_eq!(json["regions"][1]["root"]["children"][0]["id"], "group:4");
    }

    #[test]
    fn load_requires_json() {
        let layout = default_layout();
        assert!(
            layout
                .validate(&LayoutCommand::Load { json: "{}".into() })
                .is_ok()
        );
        assert_eq!(
            layout
                .validate(&LayoutCommand::Load {
                    json: "nope".into()
                })
                .unwrap_err()
                .code,
            "invalid_layout"
        );
    }
}
