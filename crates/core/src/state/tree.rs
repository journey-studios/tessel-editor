//! The live component tree exposed to automation.
//!
//! It is rebuilt from [`EditorState`] on every request, so it can never describe
//! a control the window is not showing or a value the window does not hold.
//!
//! ID scheme: `region:*` panels, `<kind>:<id>` for entities (`folder:`, `media:`,
//! `property:`, `track:`, `clip:`) and dotted names for singletons
//! (`media.search`, `inspector.tabs`, `timeline.zoom`).

use std::collections::HashMap;

use serde_json::{Map, Value, json};

use super::inspector::{
    InspectorTab, PropertyDef, PropertyKind, SECTIONS, section_def, section_properties,
};
use super::layout::{Layout, Node as LayoutNode, PanelKind};
use super::timeline::{ClipKind, TrackKind, ZOOM_MAX, ZOOM_MIN};
use super::{EditorState, WorkspaceTab};
use crate::plan::PreviewIntent;
use crate::preview::PreviewMode;

/// Window-dependent facts the pure state cannot know.
pub struct TreeContext<'a> {
    pub theme: &'a str,
    /// Region ID to `[x, y, width, height]`, measured at the last paint.
    pub metrics: &'a HashMap<String, [f32; 4]>,
    /// The live dock, snapshotted from the `DockArea` for this request.
    pub layout: &'a Layout,
    /// Whether the debug performance meter is shown (F049): window state like the theme.
    pub perf_meter: bool,
}

#[derive(Clone)]
struct Node {
    id: String,
    kind: &'static str,
    label: String,
    parent: Option<String>,
    enabled: bool,
    visible: bool,
    reason: Option<&'static str>,
    state: Map<String, Value>,
    fields: Vec<Value>,
    actions: Vec<&'static str>,
}

impl Node {
    fn new(id: impl Into<String>, kind: &'static str, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind,
            label: label.into(),
            parent: None,
            enabled: true,
            visible: true,
            reason: None,
            state: Map::new(),
            fields: Vec::new(),
            actions: Vec::new(),
        }
    }

    fn parent(mut self, parent: &str) -> Self {
        self.parent = Some(parent.to_string());
        self
    }

    fn state(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.state.insert(key.to_string(), value.into());
        self
    }

    fn visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    fn disabled(mut self, reason: &'static str) -> Self {
        self.enabled = false;
        self.reason = Some(reason);
        self
    }

    fn field(mut self, name: &str, kind: &str, extra: Value) -> Self {
        let mut field = json!({ "field": name, "type": kind });
        if let (Some(target), Some(source)) = (field.as_object_mut(), extra.as_object()) {
            target.extend(source.clone());
        }
        self.fields.push(field);
        self
    }

    fn action(mut self, name: &'static str) -> Self {
        self.actions.push(name);
        self
    }

    /// A disabled node advertises its actions as disabled too, with the
    /// node's `disabled_reason`, instead of hiding them.
    fn to_json(&self, children: Vec<String>, bounds: Option<&[f32; 4]>) -> Value {
        let actions: Vec<Value> = self
            .actions
            .iter()
            .map(|action| json!({ "action": action, "enabled": self.enabled }))
            .collect();
        let mut value = json!({
            "id": self.id, "kind": self.kind, "label": self.label, "parent": self.parent,
            "children": children, "enabled": self.enabled, "visible": self.visible,
            "state": self.state, "fields": self.fields, "actions": actions,
        });
        if let Some(reason) = self.reason {
            value["disabled_reason"] = json!(reason);
        }
        if let Some([x, y, width, height]) = bounds {
            value["bounds"] = json!({ "x": x, "y": y, "width": width, "height": height });
        }
        value
    }
}

const NO_PLAYBACK: &str = "Playback is not implemented";
const NO_TOOL: &str = "This editing tool is not implemented";

pub fn nodes(state: &EditorState, cx: &TreeContext) -> Vec<Value> {
    let built = build(state, cx);
    built
        .iter()
        .map(|node| {
            let children: Vec<String> = built
                .iter()
                .filter(|child| child.parent.as_deref() == Some(&node.id))
                .map(|child| child.id.clone())
                .collect();
            node.to_json(children, cx.metrics.get(&node.id))
        })
        .collect()
}

pub fn find(state: &EditorState, cx: &TreeContext, id: &str) -> Option<Value> {
    nodes(state, cx).into_iter().find(|node| node["id"] == id)
}

/// Every operation currently available, one entry per (node, action).
pub fn actions(state: &EditorState, cx: &TreeContext) -> Vec<Value> {
    nodes(state, cx)
        .into_iter()
        .flat_map(|node| {
            let id = node["id"].clone();
            let reason = node["disabled_reason"].clone();
            node["actions"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(move |action| {
                    let mut entry = json!({ "id": id, "action": action["action"],
                        "enabled": action["enabled"] });
                    if action["enabled"] == false {
                        entry["reason"] = reason.clone();
                    }
                    entry
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn toolbar_button(id: &str, label: &str, selected: bool) -> Node {
    Node::new(id, "icon_button", label)
        .parent("region:toolbar")
        .state("selected", selected)
        .action("toggle")
}

fn build(state: &EditorState, cx: &TreeContext) -> Vec<Node> {
    let mut nodes = vec![
        Node::new("app", "app", "Tessel Editor")
            .state("demonstration", true)
            .state("single_monitor", true),
    ];
    nodes.push(
        Node::new("region:header", "region", "Header")
            .parent("app")
            .state("fixed", true),
    );
    nodes.push(
        Node::new("region:toolbar", "region", "Toolbar")
            .parent("region:header")
            .state("fixed", true),
    );
    nodes.push(
        Node::new("region:status", "region", "Status bar")
            .parent("app")
            .state("fixed", true),
    );
    for panel in PanelKind::ALL {
        let visible = cx.layout.is_visible(panel);
        nodes.push(
            Node::new(
                format!("region:{}", panel.id()),
                "region",
                panel_label(panel),
            )
            .parent("app")
            .visible(visible)
            .field("visible", "boolean", json!({}))
            .state("visible", visible),
        );
    }
    for panel in PanelKind::ALL {
        let visible = cx.layout.is_visible(panel);
        nodes.push(toolbar_button(
            &format!("toolbar.panel.{}", panel.id()),
            &format!("Toggle {}", panel.id()),
            visible,
        ));
    }
    nodes.push(
        Node::new(
            "toolbar.layout.reset",
            "icon_button",
            "Reset the dock layout",
        )
        .parent("region:toolbar")
        .action("invoke"),
    );
    nodes.push(
        toolbar_button("toolbar.theme", "Toggle theme", cx.theme == "light")
            .state("theme", cx.theme),
    );
    nodes.push(
        toolbar_button(
            crate::perf::TARGET,
            "Debug performance meter",
            cx.perf_meter,
        )
        .field("visible", "boolean", json!({}))
        .state("visible", cx.perf_meter)
        .state("shortcut", crate::perf::SHORTCUT),
    );
    build_dock(cx.layout, &mut nodes);

    build_library(state, &mut nodes);
    build_monitor(state, &mut nodes);
    build_timeline(state, &mut nodes);
    build_inspector(state, &mut nodes);

    nodes.push(
        Node::new("status.summary", "status", state.status_summary())
            .parent("region:status")
            .state("duration_frames", state.timeline.length)
            .state("clips", state.timeline.clips.len())
            .state("fps", state.timeline.fps())
            .state(
                "resolution",
                format!("{}x{}", state.timeline.width, state.timeline.height),
            )
            .state("demonstration", state.demonstration)
            .state(
                "selection",
                state
                    .selected_clip()
                    .map(|clip| format!("clip:{}", clip.id)),
            ),
    );
    nodes
}

pub fn panel_label(panel: PanelKind) -> &'static str {
    match panel {
        PanelKind::Library => "Library",
        PanelKind::Monitor => "Monitor",
        PanelKind::Timeline => "Timeline",
        PanelKind::Inspector => "Inspector",
    }
}

/// `dock`, `dock:<region>`, `group:<n>`, `split:<n>` and `panel:<id>`: the live
/// `DockArea` as nodes. Group and split IDs live as long as their container.
fn build_dock(layout: &Layout, nodes: &mut Vec<Node>) {
    nodes.push(
        Node::new("dock", "dock_area", "Dock")
            .parent("app")
            .state(
                "hidden",
                json!(
                    layout
                        .hidden()
                        .into_iter()
                        .map(PanelKind::id)
                        .collect::<Vec<_>>()
                ),
            )
            .state("single_monitor", true)
            .action("reset"),
    );
    for state in &layout.regions {
        let dock_id = format!("dock:{}", state.region.id());
        let mut dock = Node::new(&dock_id, "dock", format!("{} dock", state.region.id()))
            .parent("dock")
            .state("open", state.open)
            .state("size", state.size);
        if state.region.is_dock() {
            dock = dock
                .field(
                    "size",
                    "number",
                    json!({ "min": super::layout::MIN_PANE,
                        "max": layout.max_dock_size(state.region), "out_of_range": "rejected" }),
                )
                .field("open", "boolean", json!({}));
        }
        nodes.push(dock);
        push_layout_node(&state.root, &dock_id, nodes);
    }
    for panel in PanelKind::ALL {
        let id = format!("panel:{}", panel.id());
        let placement = layout.locate(panel);
        let parent = placement.map_or("dock".to_string(), |found| format!("group:{}", found.group));
        let mut node = Node::new(&id, "dock_panel", panel_label(panel))
            .parent(&parent)
            .visible(layout.is_visible(panel))
            .state("visible", layout.is_visible(panel))
            .state("region", placement.map(|found| found.region.id()))
            .state(
                "group",
                placement.map(|found| format!("group:{}", found.group)),
            )
            .state("index", placement.map(|found| found.index))
            .state("active", placement.is_some_and(|found| found.active))
            .field("visible", "boolean", json!({}))
            .action("hide")
            .action("show");
        if placement.is_some() {
            node = node.action("activate");
        }
        nodes.push(node);
    }
}

fn push_layout_node(node: &LayoutNode, parent: &str, nodes: &mut Vec<Node>) {
    match node {
        LayoutNode::Split {
            axis,
            sizes,
            children,
            ..
        } => {
            let id = node.dom_id();
            nodes.push(
                Node::new(&id, "dock_split", "Dock split")
                    .parent(parent)
                    .state("axis", axis.id())
                    .state("sizes", json!(sizes))
                    .field(
                        "sizes",
                        "number_list",
                        json!({ "count": children.len(), "min": super::layout::MIN_PANE }),
                    ),
            );
            for child in children {
                push_layout_node(child, &id, nodes);
            }
        }
        LayoutNode::Group { active, panels, .. } => {
            nodes.push(
                Node::new(node.dom_id(), "dock_group", "Dock tab group")
                    .parent(parent)
                    .state("active", *active)
                    .state(
                        "panels",
                        json!(
                            panels
                                .iter()
                                .map(|panel| format!("panel:{}", panel.id()))
                                .collect::<Vec<_>>()
                        ),
                    ),
            );
        }
    }
}

fn build_library(state: &EditorState, nodes: &mut Vec<Node>) {
    let library = &state.library;
    nodes.push(
        Node::new("library.bins", "tree", "Project bins")
            .parent("region:library")
            .state(
                "selected",
                library.selected_folder.map(|id| format!("folder:{id}")),
            ),
    );
    nodes.push(
        Node::new("folder:all", "folder", "All media")
            .parent("library.bins")
            .state("count", library.items.len())
            .state("depth", 0)
            .state("selected", library.selected_folder.is_none())
            .action("select"),
    );
    for folder in &library.folders {
        let id = format!("folder:{}", folder.id);
        let parent = folder.parent.map_or("library.bins".to_string(), |parent| {
            format!("folder:{parent}")
        });
        let expandable = library.has_children(folder.id);
        let mut node = Node::new(&id, "folder", folder.label.as_str())
            .parent(&parent)
            .visible(library.is_visible(folder.id))
            .state("icon", folder.icon.label())
            .state("depth", library.depth(folder.id))
            .state("count", library.count(folder.id))
            .state("expandable", expandable)
            .state("selected", library.selected_folder == Some(folder.id))
            .action("select");
        if expandable {
            node = node
                .state("expanded", folder.expanded)
                .field("expanded", "boolean", json!({}))
                .action("toggle");
        }
        nodes.push(node);
    }
    let results = library.results();
    nodes.push(
        Node::new("media.search", "search", "Search media")
            .parent("region:library")
            .state("value", library.search.clone())
            .state("bin", library.bin_label())
            .state("results", results.len())
            .state(
                "result_ids",
                results
                    .iter()
                    .map(|item| format!("media:{}", item.id))
                    .collect::<Vec<_>>(),
            )
            .field("value", "string", json!({})),
    );
    nodes.push(
        Node::new("media.grid", "grid", library.bin_label())
            .parent("region:library")
            .state("count", results.len()),
    );
    for item in &library.items {
        let shown = results.iter().any(|result| result.id == item.id);
        nodes.push(
            Node::new(format!("media:{}", item.id), "media", item.name.as_str())
                .parent("media.grid")
                .visible(shown)
                .state("code", item.code.as_str())
                .state("meta", item.meta.as_str())
                .state("media_kind", item.kind.label())
                .state("folder", format!("folder:{}", item.folder))
                .state("demo_image", item.image.is_some())
                .state("selected", state.selected_media == Some(item.id))
                .action("select"),
        );
    }
    nodes.push(
        Node::new("media.insert", "button", "Insert into timeline")
            .parent("region:library")
            .disabled("Inserting media is not implemented")
            .action("invoke"),
    );
}

fn build_monitor(state: &EditorState, nodes: &mut Vec<Node>) {
    let media = state.subject_media();
    let playhead = state.timeline.playhead;
    nodes.push(
        Node::new(
            "monitor",
            "monitor",
            media.map_or("No image", |item| item.name.as_str()),
        )
        .parent("region:monitor")
        .state("media", media.map(|item| format!("media:{}", item.id)))
        .state("media_code", media.map(|item| item.code.as_str()))
        .state("demo_image", media.is_some_and(|item| item.image.is_some()))
        .state("subject", format!("{:?}", state.subject).to_lowercase())
        .state("timecode", state.timeline.timecode(playhead))
        .state("frame", playhead)
        .state("duration", state.timeline.timecode(state.timeline.length))
        .state("fps", state.timeline.fps())
        .state("preview", state.preview.to_json().to_string())
        .state("playback", "not implemented"),
    );
    for (id, label, action) in [
        ("monitor.go_start", "Go to start", "invoke"),
        ("monitor.step_back", "Step back one frame", "invoke"),
        ("monitor.step_forward", "Step forward one frame", "invoke"),
        ("monitor.go_end", "Go to end", "invoke"),
    ] {
        nodes.push(
            Node::new(id, "icon_button", label)
                .parent("region:monitor")
                .action(action),
        );
    }
    nodes.push(
        Node::new("monitor.play", "icon_button", "Play")
            .parent("region:monitor")
            .disabled(NO_PLAYBACK)
            .action("invoke"),
    );
    let sequence = matches!(state.preview, PreviewIntent::Sequence);
    nodes.push(
        Node::new(
            "monitor.preview.sequence",
            "toggle_button",
            "Preview the sequence",
        )
        .parent("region:monitor")
        .state("selected", sequence)
        .action("invoke"),
    );
    let mut source = Node::new(
        "monitor.preview.source",
        "toggle_button",
        "Preview the source",
    )
    .parent("region:monitor")
    .state("selected", !sequence)
    .action("invoke");
    if state.previewable_media().is_none() {
        source = source.disabled("Select a media item with a file to preview its source");
    }
    nodes.push(source);
    // The policy stays visible in the tree: which quality playback and scrub draw is never hidden.
    nodes.push(
        Node::new(
            "monitor.preview.quality",
            "toggle_button",
            "Fast preview (proxy and lighter MSAA while playing or scrubbing)",
        )
        .parent("region:monitor")
        .state("selected", state.preview_policy.mode == PreviewMode::Auto)
        .state("policy", state.preview_policy.to_json().to_string())
        .action("invoke"),
    );
    nodes.push(
        Node::new("monitor.reload", "icon_button", "Reload the viewport")
            .parent("region:monitor")
            .action("invoke"),
    );
    // F055: the translation gizmo is drawn and hit on a canvas inside the page, so it has no
    // control of its own to click: this node shows what the core holds for it, and the semantic
    // way to drive it is `scene.pointer.*` / `scene.translate` (`ui op`, `scripts/headless`).
    // Only with a scene attached, so the tree of a project without scenes is what it always was.
    if !state.scenes.is_empty() {
        let drag = state.scene_interaction.drag();
        nodes.push(
            Node::new("monitor.gizmo", "gizmo", "Translation gizmo (canvas)")
                .parent("region:monitor")
                .state(
                    "selection",
                    state.scene_interaction.selection().entries().len() as u64,
                )
                .state("dragging", drag.is_some())
                .state("handle", drag.map_or("", |drag| drag.handle().as_str()))
                .state("gesture", drag.map_or("", |drag| drag.gesture())),
        );
    }
}

fn build_timeline(state: &EditorState, nodes: &mut Vec<Node>) {
    let timeline = &state.timeline;
    let edit = state.workspace_tab == WorkspaceTab::Edit;
    nodes.push(
        Node::new("workspace.tabs", "tabs", "Workspace mode")
            .parent("region:timeline")
            .state("value", state.workspace_tab.id())
            .state(
                "options",
                WorkspaceTab::ALL.iter().map(|tab| tab.id()).collect::<Vec<_>>(),
            )
            .state("implemented", json!(["edit"]))
            .field(
                "value",
                "choice",
                json!({ "options": WorkspaceTab::ALL.iter().map(|tab| tab.id()).collect::<Vec<_>>() }),
            ),
    );
    nodes.push(
        Node::new("timeline.tool.select", "icon_button", "Selection tool")
            .parent("region:timeline")
            .state("selected", true)
            .disabled("The selection tool is the only tool and is always active"),
    );
    for (id, label) in [
        ("timeline.tool.razor", "Razor tool"),
        ("timeline.tool.delete", "Delete clip"),
    ] {
        nodes.push(
            Node::new(id, "icon_button", label)
                .parent("region:timeline")
                .disabled(NO_TOOL)
                .action("invoke"),
        );
    }
    let sequence_ids: Vec<&str> = state.all_sequences().map(|t| t.id.as_str()).collect();
    nodes.push(
        Node::new("timeline.sequence", "select", "Sequence")
            .parent("region:timeline")
            .state("value", timeline.id.as_str())
            .state("options", sequence_ids.clone())
            // Every sequence keeps its own settings, so the selector can show what a switch lands on.
            .state(
                "sequences",
                state
                    .all_sequences()
                    .map(|sequence| {
                        json!({
                            "id": sequence.id, "name": sequence.name,
                            "width": sequence.width, "height": sequence.height,
                            "length": sequence.length, "fps": sequence.fps(),
                            "active": sequence.id == timeline.id,
                        })
                    })
                    .collect::<Vec<_>>(),
            )
            .field("value", "choice", json!({ "options": sequence_ids })),
    );
    nodes.push(
        Node::new("timeline.snap", "toggle", "Snap")
            .parent("region:timeline")
            .visible(edit)
            .state("value", timeline.snap)
            .field("value", "boolean", json!({}))
            .action("toggle"),
    );
    nodes.push(
        Node::new("timeline.zoom", "slider", "Timeline zoom")
            .parent("region:timeline")
            .visible(edit)
            .state("value", timeline.zoom)
            .state("unit", "px/s")
            .state("min", ZOOM_MIN)
            .state("max", ZOOM_MAX)
            .field("value", "number", json!({ "min": ZOOM_MIN, "max": ZOOM_MAX, "unit": "px/s", "out_of_range": "clamped" })),
    );
    nodes.push(
        Node::new("timeline.playhead", "playhead", "Playhead")
            .parent("region:timeline")
            .visible(edit)
            .state("frame", timeline.playhead)
            .state("timecode", timeline.timecode(timeline.playhead))
            .state("fps", timeline.fps())
            .state("min", 0)
            .state("max", timeline.length)
            .field(
                "frame",
                "integer",
                json!({ "min": 0, "max": timeline.length, "out_of_range": "clamped" }),
            ),
    );
    for track in &timeline.tracks {
        let id = format!("track:{}", track.id);
        let mut node = Node::new(&id, "track", track.name.as_str())
            .parent("region:timeline")
            .visible(edit)
            .state("label", timeline.label(track.id))
            .state("track_kind", track.kind.label())
            .state(
                "order",
                timeline.tracks.iter().position(|t| t.id == track.id),
            )
            .state("selected", timeline.selected_track == Some(track.id))
            .action("select");
        node = match track.kind {
            TrackKind::Video => node
                .state("hidden", track.hidden)
                .field("hidden", "boolean", json!({}))
                .action("toggle_hidden"),
            TrackKind::Audio => node
                .state("muted", track.muted)
                .field("muted", "boolean", json!({}))
                .action("toggle_muted"),
        };
        nodes.push(node.state("clips", timeline.clips_on(&track.id).count()));
    }
    for clip in &timeline.clips {
        nodes.push(
            Node::new(format!("clip:{}", clip.id), "clip", clip.name.as_str())
                .parent(&format!("track:{}", clip.track))
                .visible(edit)
                .state("clip_kind", clip.kind.label())
                .state("track", format!("track:{}", clip.track))
                .state("start", clip.start)
                .state("duration", clip.duration)
                .state("end", clip.end())
                .state("start_timecode", timeline.timecode(clip.start))
                .state("media", clip.media.map(|media| format!("media:{media}")))
                .state(
                    "waveform",
                    (clip.kind == ClipKind::Audio).then_some("illustrative demonstration data"),
                )
                .state("selected", timeline.selected_clip == Some(clip.id))
                .action("select"),
        );
        for (side, frame) in [("start", clip.start), ("end", clip.end())] {
            nodes.push(
                Node::new(
                    format!("trim:{}:{side}", clip.id),
                    "trim_handle",
                    format!("Trim {side}"),
                )
                .parent(&format!("clip:{}", clip.id))
                .visible(edit)
                .state("frame", frame)
                .state("edge", side)
                .field(
                    "frame",
                    "integer",
                    json!({"min":0,"max":timeline.length,"unit":"frames"}),
                ),
            );
        }
    }
}

fn build_inspector(state: &EditorState, nodes: &mut Vec<Node>) {
    let inspector = &state.inspector;
    let clip = state.inspector_clip();
    nodes.push(
        Node::new("inspector.tabs", "tabs", "Inspector tabs")
            .parent("region:inspector")
            .state("value", inspector.tab.id())
            .state(
                "options",
                InspectorTab::ALL.iter().map(|tab| tab.id()).collect::<Vec<_>>(),
            )
            .field(
                "value",
                "choice",
                json!({ "options": InspectorTab::ALL.iter().map(|tab| tab.id()).collect::<Vec<_>>() }),
            ),
    );
    nodes.push(
        Node::new("inspector.thumbnail", "thumbnail", "Preview")
            .parent("region:inspector")
            .state(
                "media",
                state
                    .subject_media()
                    .map(|item| format!("media:{}", item.id)),
            )
            .state("clip", clip.map(|clip| format!("clip:{}", clip.id)))
            .state("guides", "rule of thirds"),
    );
    for section in SECTIONS {
        let applicable = clip.is_some_and(|clip| section.applies.includes(clip.kind));
        let in_tab = section.tab == inspector.tab;
        let id = format!("inspector.{}", section.id);
        let modified = clip.is_some_and(|clip| {
            inspector.section_modified(clip.id, section.id)
                || (section.id == "camera" && !clip.camera_keys.is_empty())
        });
        let mut node = Node::new(&id, "accordion", section.label)
            .parent("region:inspector")
            .visible(in_tab && applicable)
            .state("expanded", inspector.is_expanded(section.id))
            .state("tab", section.tab.id())
            .state("modified", modified)
            .field("expanded", "boolean", json!({}))
            .action("toggle");
        node = if applicable {
            node.action("reset")
        } else {
            node
        };
        nodes.push(node);
        for property in section_properties(section.id) {
            nodes.push(property_node(
                state,
                &id,
                property,
                in_tab && applicable && inspector.is_expanded(section.id),
            ));
        }
        if let (true, Some(clip)) = (section.id == "camera" && applicable, clip) {
            let shown = in_tab && inspector.is_expanded(section.id);
            build_camera_keys(state, clip, &id, shown, nodes);
        }
    }
}

const NO_KEYS: &str = "The clip has no camera keys";
const PLAYHEAD_OUTSIDE: &str = "Move the playhead over the clip to key its camera";
const ORBIT_CONFLICT: &str =
    "The camera orbits: set `orbit` to 0 (clip.camera) before keying it; keys replace the orbit";

/// Camera keyframe controls of the selected scene clip (BL140): the capture/clear buttons and one
/// node per key whose fields `set` edits and whose `remove` action deletes it.
fn build_camera_keys(
    state: &EditorState,
    clip: &super::timeline::Clip,
    parent: &str,
    shown: bool,
    nodes: &mut Vec<Node>,
) {
    let playhead = state.timeline.playhead;
    let inside = playhead >= clip.start && playhead < clip.end();
    let mut capture = Node::new("camera_keys.capture", "button", "Add key at playhead")
        .parent(parent)
        .visible(shown)
        .state("clip", format!("clip:{}", clip.id))
        .state("playhead", playhead)
        .state(
            "scene_time",
            state.clip_scene_time(clip, playhead).to_json(),
        )
        .action("invoke");
    // The same refusals `Capture` applies, shown before the click instead of as a failing callback.
    let orbiting = clip.camera.is_some_and(|camera| camera.orbit != 0.0);
    if orbiting {
        capture = capture.disabled(ORBIT_CONFLICT);
    } else if !inside {
        capture = capture.disabled(PLAYHEAD_OUTSIDE);
    }
    nodes.push(capture);
    let mut clear = Node::new("camera_keys.clear", "button", "Remove all camera keys")
        .parent(parent)
        .visible(shown)
        .state("count", clip.camera_keys.len())
        .action("invoke");
    if clip.camera_keys.is_empty() {
        clear = clear.disabled(NO_KEYS);
    }
    nodes.push(clear);
    for (index, key) in clip.camera_keys.iter().enumerate() {
        let mut node = Node::new(
            format!("camera_key:{}:{index}", clip.id),
            "camera-key",
            format!("Camera key {} s", key.time),
        )
        .parent(parent)
        .visible(shown)
        .state("clip", format!("clip:{}", clip.id))
        .state("index", index)
        .state("time", key.time.to_json())
        .state("position", json!(key.position))
        .state("target", json!(key.target))
        .state("fov", key.fov)
        .state("easing", key.easing.label())
        .field(
            "time",
            "number",
            json!({ "unit": "seconds", "resolution": 0.001 }),
        )
        .field(
            "easing",
            "choice",
            json!({ "options": crate::camera_keys::Easing::labels() }),
        )
        .field(
            "fov",
            "number",
            json!({ "min": 1, "max": 170, "max_exclusive": true, "unit": "degrees" }),
        )
        .action("remove");
        for axis in ["x", "y", "z"] {
            node = node
                .field(&format!("position_{axis}"), "number", json!({}))
                .field(&format!("target_{axis}"), "number", json!({}));
        }
        nodes.push(node);
    }
}

fn property_node(state: &EditorState, parent: &str, property: &PropertyDef, visible: bool) -> Node {
    let inspector = &state.inspector;
    let clip = state.inspector_clip();
    let applicable = clip.is_some_and(|clip| {
        section_def(property.section).is_some_and(|section| section.applies.includes(clip.kind))
    });
    let mut node = Node::new(
        format!("property:{}", property.id),
        "property",
        property.label,
    )
    .parent(parent)
    .visible(visible)
    .state("section", property.section)
    .state("default", property.default.to_json());
    let editable = applicable && clip.is_some_and(|clip| inspector.is_enabled(clip.id, property));
    node = match (clip, applicable) {
        (Some(clip), true) => node
            .state("value", inspector.value(clip.id, property).to_json())
            .state("modified", inspector.is_modified(clip.id, property))
            .state("clip", format!("clip:{}", clip.id)),
        _ => node.state("value", Value::Null),
    };
    node = match property.kind {
        PropertyKind::Slider {
            min,
            max,
            step,
            decimals,
            unit,
        } => node
            .state("type", "slider")
            .state("min", min)
            .state("max", max)
            .state("step", step)
            .state("decimals", decimals)
            .state("unit", unit)
            // True while a drag (or `property.gesture.*`) previews this slider: the value is
            // applied but its undo entry is only recorded when the gesture ends.
            .state(
                "gesture_open",
                state
                    .open_property_gesture()
                    .is_some_and(|open| open.property == property.id),
            )
            .field(
                "value",
                "number",
                json!({ "min": min, "max": max, "unit": unit, "out_of_range": "clamped" }),
            ),
        PropertyKind::Select { options } => node
            .state("type", "select")
            .state("options", options)
            .field("value", "choice", json!({ "options": options })),
        PropertyKind::Toggle => node
            .state("type", "toggle")
            .field("value", "boolean", json!({}))
            .action("toggle"),
    };
    if !applicable {
        return node.disabled("No clip of a matching kind is selected");
    }
    if !editable {
        let reason = match property.requires {
            Some("ken_burns") => "Turn Ken Burns on to edit this value",
            _ => "Unavailable",
        };
        return node.disabled(reason).action("reset");
    }
    node.action("reset")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::PreviewIntent;
    use crate::state::layout::default_layout;
    use crate::state::{Command, ProjectSettings};
    use crate::{CommandError, Raw};

    fn node(state: &EditorState, id: &str) -> Value {
        let layout = default_layout();
        let metrics = HashMap::new();
        let cx = TreeContext {
            theme: "dark",
            metrics: &metrics,
            layout: &layout,
            perf_meter: false,
        };
        find(state, &cx, id).unwrap_or_else(|| panic!("{id} is missing from the tree"))
    }

    /// Sets `timeline.sequence` the way a window, `ui set` or a script does.
    fn select_sequence(state: &mut EditorState, id: &str) -> Result<(), CommandError> {
        match crate::commands::parse_set("timeline.sequence", "value", &Raw::Text(id.into()))? {
            crate::commands::Parsed::Run(command) => state.execute(command).map(|_| ()),
            _ => unreachable!("selecting a sequence is a state command"),
        }
    }

    #[test]
    fn the_gizmo_node_exists_only_with_a_scene_and_follows_the_drag() {
        use std::path::Path;
        let layout = default_layout();
        let metrics = HashMap::new();
        let cx = TreeContext {
            theme: "dark",
            metrics: &metrics,
            layout: &layout,
            perf_meter: false,
        };
        let has =
            |state: &EditorState| nodes(state, &cx).iter().any(|n| n["id"] == "monitor.gizmo");
        // A project without scenes has the tree it always had.
        assert!(!has(&EditorState::demo()));

        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json");
        let bundle: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let mut state = EditorState::demo();
        let run = |state: &mut EditorState, request: Value| {
            crate::headless::execute(state, &request).unwrap_or_else(|e| panic!("{request}: {e:?}"))
        };
        run(&mut state, json!({"op":"project.create","name":"Tree"}));
        run(
            &mut state,
            json!({"op":"scene.attach","id":"street","bundle":bundle}),
        );
        assert!(has(&state));
        let idle = node(&state, "monitor.gizmo");
        assert_eq!(idle["parent"], "region:monitor");
        assert_eq!(idle["state"]["dragging"], false);
        assert_eq!(idle["state"]["selection"], 0);

        let figure = "scene/scene-a/node/figure";
        run(
            &mut state,
            json!({"op":"scene.select","scene":"street","entities":[figure]}),
        );
        let shown = run(
            &mut state,
            json!({"op":"scene.gizmo.state","scene":"street","frame":0}),
        );
        let tip = shown["overlay"]["handles"][0]["to"].clone();
        run(
            &mut state,
            json!({"op":"scene.pointer.down","scene":"street","frame":0,"point":tip}),
        );
        let dragging = node(&state, "monitor.gizmo");
        assert_eq!(dragging["state"]["dragging"], true);
        assert_eq!(dragging["state"]["handle"], "x");
        assert_eq!(dragging["state"]["selection"], 1);
        // The summary a polling client reads shows the same drag.
        assert_eq!(state.summary()["scene_interaction"]["drag"]["handle"], "x");

        run(
            &mut state,
            json!({"op":"scene.pointer.cancel","reason":"escape"}),
        );
        let after = node(&state, "monitor.gizmo");
        assert_eq!(after["state"]["dragging"], false);
        assert_eq!(after["state"]["handle"], "");
        assert!(state.summary()["scene_interaction"]["drag"].is_null());
    }

    #[test]
    fn camera_key_nodes_exist_only_for_the_selected_scene_clip() {
        use crate::fixtures::{proof_requests, write_standard_set};
        let dir = std::env::temp_dir().join(format!("tessel-tree-camkeys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let set = write_standard_set(&dir).unwrap();
        let mut state = EditorState::demo();
        for request in proof_requests(&set) {
            crate::headless::execute(&mut state, &request).unwrap();
        }
        let layout = default_layout();
        let metrics = HashMap::new();
        let cx = TreeContext {
            theme: "dark",
            metrics: &metrics,
            layout: &layout,
            perf_meter: false,
        };
        let ids = |state: &EditorState| -> Vec<String> {
            nodes(state, &cx)
                .iter()
                .filter_map(|node| node["id"].as_str().map(str::to_string))
                .filter(|id| id.starts_with("camera_key"))
                .collect()
        };
        for request in [
            json!({"op":"invoke","target":"clip:c-img","action":"select"}),
            json!({"op":"clip.camera","clip":"c-scene","position":[0,1.2,5],"orbit":0}),
        ] {
            crate::headless::execute(&mut state, &request).unwrap();
        }
        assert!(
            ids(&state).is_empty(),
            "an image clip has no camera section"
        );
        assert_eq!(node(&state, "inspector.camera")["visible"], false);
        crate::headless::execute(
            &mut state,
            &json!({"op":"invoke","target":"clip:c-scene","action":"select"}),
        )
        .unwrap();
        assert_eq!(node(&state, "inspector.camera")["visible"], true);
        assert_eq!(ids(&state), ["camera_keys.capture", "camera_keys.clear"]);
        crate::headless::execute(
            &mut state,
            &json!({"op":"clip.camera_key","clip":"c-scene","time":1,"position":[0,1,5],"fov":42}),
        )
        .unwrap();
        assert_eq!(
            ids(&state),
            [
                "camera_keys.capture",
                "camera_keys.clear",
                "camera_key:c-scene:0"
            ]
        );
        let key = node(&state, "camera_key:c-scene:0");
        assert_eq!(key["state"]["fov"], 42.0);
        assert_eq!(key["actions"][0]["action"], "remove");
        assert_eq!(node(&state, "inspector.camera")["state"]["modified"], true);
        assert_eq!(node(&state, "camera_keys.clear")["enabled"], true);
    }

    #[test]
    fn capture_is_disabled_with_a_reason_while_the_camera_orbits() {
        use crate::fixtures::{proof_requests, write_standard_set};
        let dir = std::env::temp_dir().join(format!("tessel-tree-orbit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let set = write_standard_set(&dir).unwrap();
        let mut state = EditorState::demo();
        for request in proof_requests(&set) {
            crate::headless::execute(&mut state, &request).unwrap();
        }
        crate::headless::execute(
            &mut state,
            &json!({"op":"invoke","target":"clip:c-scene","action":"select"}),
        )
        .unwrap();
        // The proof scene clip orbits (45 deg/s) and the playhead is over it.
        let capture = node(&state, "camera_keys.capture");
        assert_eq!(capture["enabled"], false);
        assert!(
            capture["disabled_reason"]
                .as_str()
                .unwrap()
                .contains("orbit"),
            "{capture}"
        );
        // The refusal the button announces is the one the command gives.
        let refused = crate::headless::execute(
            &mut state,
            &json!({"op":"invoke","target":"camera_keys.capture","action":"invoke"}),
        )
        .unwrap_err();
        assert_eq!(refused.code, "camera_orbit_conflict");
        crate::headless::execute(
            &mut state,
            &json!({"op":"clip.camera","clip":"c-scene","position":[0,1.2,5],"orbit":0}),
        )
        .unwrap();
        assert_eq!(node(&state, "camera_keys.capture")["enabled"], true);
    }

    #[test]
    fn the_sequence_selector_lists_every_sequence_and_switching_keeps_each_ones_work() {
        let mut state = EditorState::demo();
        let clips = state.timeline.clips.len();
        crate::headless::execute(
            &mut state,
            &json!({"op":"sequence.add","id":"vertical","name":"Vertical","width":180,
                "height":320,"rate":30,"length":60,"activate":false}),
        )
        .unwrap();
        crate::headless::execute(
            &mut state,
            &json!({"op":"set","target":"timeline.zoom","field":"value","value":40}),
        )
        .unwrap();
        let (width, height, length) = (
            state.timeline.width,
            state.timeline.height,
            state.timeline.length,
        );

        let selector = node(&state, "timeline.sequence");
        assert_eq!(selector["parent"], "region:timeline");
        assert_eq!(selector["state"]["value"], state.timeline.id.as_str());
        assert_eq!(selector["state"]["options"].as_array().unwrap().len(), 2);
        let listed = selector["state"]["sequences"].as_array().unwrap();
        let vertical = listed.iter().find(|s| s["id"] == "vertical").unwrap();
        assert_eq!(
            (
                vertical["width"].as_u64(),
                vertical["height"].as_u64(),
                vertical["fps"].as_i64()
            ),
            (Some(180), Some(320), Some(30))
        );

        select_sequence(&mut state, "vertical").unwrap();
        assert_eq!(state.timeline.id, "vertical");
        assert_eq!((state.timeline.width, state.timeline.height), (180, 320));
        assert_eq!(state.timeline.length, 60);
        assert_eq!(
            node(&state, "timeline.sequence")["state"]["value"],
            "vertical"
        );
        crate::headless::execute(
            &mut state,
            &json!({"op":"set","target":"timeline.zoom","field":"value","value":90}),
        )
        .unwrap();

        // Back on the first one: its clips, canvas, length and view are exactly as left.
        let main = state.sequences[0].id.to_string();
        select_sequence(&mut state, &main).unwrap();
        assert_eq!(state.timeline.clips.len(), clips);
        assert_eq!(
            (
                state.timeline.width,
                state.timeline.height,
                state.timeline.length
            ),
            (width, height, length)
        );
        assert_eq!(state.timeline.zoom, 40.0);
        assert_eq!(state.sequence("vertical").unwrap().zoom, 90.0);
        assert_eq!(state.sequence("vertical").unwrap().width, 180);

        let error = select_sequence(&mut state, "missing").unwrap_err();
        assert_eq!(error.code, "unknown_element");
        assert_eq!(
            state.timeline.id.as_str(),
            main,
            "a refused switch changes nothing"
        );
    }

    #[test]
    fn monitor_modes_and_reload_are_nodes_of_the_monitor_region() {
        let state = EditorState::demo();
        for id in [
            "monitor.preview.sequence",
            "monitor.preview.source",
            "monitor.reload",
        ] {
            let found = node(&state, id);
            assert_eq!(found["parent"], "region:monitor", "{id}");
            assert_eq!(found["actions"][0]["action"], "invoke", "{id}");
        }
    }

    #[test]
    fn the_perf_meter_is_a_toolbar_node_with_a_visible_field_and_its_shortcut() {
        let state = EditorState::demo();
        let found = node(&state, crate::perf::TARGET);
        assert_eq!(found["parent"], "region:toolbar");
        assert_eq!(found["actions"][0]["action"], "toggle");
        assert_eq!(found["fields"][0]["field"], "visible");
        assert_eq!(found["state"]["visible"], false);
        assert_eq!(found["state"]["shortcut"], crate::perf::SHORTCUT);
    }

    #[test]
    fn the_mode_buttons_show_which_preview_is_active() {
        let mut state = EditorState::demo();
        assert_eq!(
            node(&state, "monitor.preview.sequence")["state"]["selected"],
            true
        );
        assert_eq!(
            node(&state, "monitor.preview.source")["state"]["selected"],
            false
        );
        let asset = state.previewable_media().unwrap().id;
        state
            .execute(Command::SetPreview {
                intent: PreviewIntent::Source {
                    asset,
                    time: crate::time::Ratio::ZERO,
                },
            })
            .unwrap();
        assert_eq!(
            node(&state, "monitor.preview.sequence")["state"]["selected"],
            false
        );
        assert_eq!(
            node(&state, "monitor.preview.source")["state"]["selected"],
            true
        );
    }

    #[test]
    fn the_source_mode_is_disabled_with_a_reason_when_nothing_can_be_previewed() {
        let state = EditorState::empty(&ProjectSettings::default()).unwrap();
        let source = node(&state, "monitor.preview.source");
        assert_eq!(source["enabled"], false);
        assert!(!source["disabled_reason"].as_str().unwrap().is_empty());
        assert_eq!(source["actions"][0]["enabled"], false);
        // The sequence mode never depends on a selection.
        assert_eq!(node(&state, "monitor.preview.sequence")["enabled"], true);
    }
}
