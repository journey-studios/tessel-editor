//! State and handlers of the directed-scene inspector (BL014/BL021;
//! `docs/features/scene-inspector.md`).
//!
//! The window never keeps its own copy of a scene. The tree, the selection and every value are
//! read from `EditorState` (`scene_model.rs`), and every change is a `Command::Scene` through
//! [`Workstation::execute`]: `scene.select`, `scene.gesture.*` and `scene.edit`. What lives here
//! is presentation only: which inspector is showing, which attached scene the panel follows,
//! which tree rows are expanded, and the Kit controls that mirror the model.
//!
//! The same operations are reachable without a pointer: `ui op '{"op":"scene.select",..}'` runs
//! the identical command, and the semantic nodes below (`scene.entity:*`, `scene.property:*`)
//! answer `invoke`/`set` through the handlers the pointer uses.

use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::component::searchable_list::SearchableVec;
use gpui_kit::component::select::{SelectEvent, SelectState};
use gpui_kit::component::slider::SliderState;
use gpui_kit::component::tree::{TreeEvent, TreeState};
use gpui_kit::*;
use serde_json::{Value, json};
use tessel_editor_core::scene_edit::{
    EditOp, PropertyRef, PropertySpec, PropertyValue, read_property,
};
use tessel_editor_core::scene_interaction::{
    GestureCommand, SelectMode, entity_of_property, entity_path,
};
use tessel_editor_core::scene_project::SceneCommand;

use super::scene_fx::{self, FxUi};
use super::scene_gesture::{SceneSliderGesture, SliderKey};
use super::scene_model::{
    Availability, MAX_LISTED_ENTITIES, PropertyView, Scrub, Snapshot, SnapshotCache, Target,
    TreeEntry, component_values, components, scrub, unit_label,
};
use super::{Source, Workstation};
use crate::state::{Command, CommandError, Raw};

/// Which inspector the panel shows. Presentation state of the window, like the active dock tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectorMode {
    Clip,
    Scene,
}

impl InspectorMode {
    pub const ALL: [Self; 2] = [Self::Clip, Self::Scene];

    pub fn id(self) -> &'static str {
        match self {
            Self::Clip => "clip",
            Self::Scene => "scene",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Clip => "Clipe",
            Self::Scene => "Cena",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.id() == raw)
    }
}

/// The last typed error of a scene edit, kept on screen until the next operation succeeds so a
/// refused value is explained, not just flashed in the status bar.
#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub code: String,
    pub message: String,
}

/// A slider the panel built, with the window it was built for (see `scene_model::scrub`).
pub struct SliderEntry {
    pub state: Entity<SliderState>,
    pub scrub: Scrub,
}

/// Rows the tree shows by default: the definition and the nodes under its root. Deeper entries
/// start collapsed so a large hierarchy does not open at once.
const DEFAULT_EXPANDED_DEPTH: usize = 2;

pub struct SceneUi {
    pub mode: InspectorMode,
    /// The attached scene the panel follows; `None` follows the first one.
    chosen: Option<String>,
    cache: SnapshotCache,
    pub tree: Entity<TreeState>,
    /// Rows and selection last pushed into `tree`, so the Kit tree is rebuilt only on change
    /// (rebuilding resets its own selection and scroll).
    tree_signature: Vec<(String, bool)>,
    /// The entity the Kit tree was last told to select. A selection in the tree that differs
    /// from it came from the user (arrow keys), and becomes `scene.select`.
    pub pushed_selection: Option<String>,
    expansion: HashMap<String, bool>,
    pub picker: Entity<SelectState<SearchableVec<String>>>,
    picker_choices: Vec<(String, String)>,
    pub sliders: HashMap<SliderKey, SliderEntry>,
    pub gesture: SceneSliderGesture,
    /// The effects section's sliders and drag buffer (`scene_fx.rs`).
    pub fx: FxUi,
    pub notice: Option<Notice>,
}

impl SceneUi {
    pub fn new(window: &mut Window, cx: &mut Context<Workstation>) -> Self {
        let tree = cx.new(|cx| TreeState::new(cx));
        cx.subscribe(&tree, |this, _, event: &TreeEvent, cx| {
            let (path, expanded) = match event {
                TreeEvent::Expanded(id) => (id.to_string(), true),
                TreeEvent::Collapsed(id) => (id.to_string(), false),
            };
            this.scene_ui.expansion.insert(path, expanded);
            cx.notify();
        })
        .detach();
        // Arrow keys move the Kit tree's own selection without an event. Following it here makes
        // the keyboard and the pointer end in the same `scene.select`.
        cx.observe(&tree, |this, tree, cx| this.scene_tree_moved(&tree, cx))
            .detach();

        let picker = cx
            .new(|cx| SelectState::new(SearchableVec::new(Vec::<String>::new()), None, window, cx));
        cx.subscribe(
            &picker,
            |this, _, event: &SelectEvent<SearchableVec<String>>, cx| {
                let SelectEvent::Confirm(Some(label)) = event else {
                    return;
                };
                let Some((id, _)) = this
                    .scene_ui
                    .picker_choices
                    .iter()
                    .find(|(_, choice)| choice == label)
                else {
                    return;
                };
                this.scene_ui.chosen = Some(id.clone());
                cx.notify();
            },
        )
        .detach();

        Self {
            mode: InspectorMode::Clip,
            chosen: None,
            cache: SnapshotCache::default(),
            tree,
            tree_signature: Vec::new(),
            pushed_selection: None,
            expansion: HashMap::new(),
            picker,
            picker_choices: Vec::new(),
            sliders: HashMap::new(),
            gesture: SceneSliderGesture::default(),
            fx: FxUi::default(),
            notice: None,
        }
    }

    pub fn is_expanded(&self, path: &str, depth: usize) -> bool {
        self.expansion
            .get(path)
            .copied()
            .unwrap_or(depth < DEFAULT_EXPANDED_DEPTH)
    }
}

/// The tree as the Kit consumes it: only expanded subtrees carry their children.
fn tree_item(ui: &SceneUi, entry: &TreeEntry, depth: usize) -> gpui_kit::component::tree::TreeItem {
    let expanded = ui.is_expanded(&entry.path, depth);
    gpui_kit::component::tree::TreeItem::new(entry.path.clone(), entry.label())
        .expanded(expanded)
        .children(
            entry
                .children
                .iter()
                .map(|child| tree_item(ui, child, depth + 1)),
        )
}

fn notice_of(error: &CommandError) -> Notice {
    Notice {
        code: error.code.to_string(),
        message: error.message.clone(),
    }
}

impl Workstation {
    // ---- reads ----

    /// The attached scene the panel follows: the chosen one while it is attached, else the first.
    pub(super) fn scene_active_id(&self) -> Option<String> {
        let ids = self.state.scenes.ids();
        self.scene_ui
            .chosen
            .as_ref()
            .filter(|chosen| ids.contains(&chosen.as_str()))
            .cloned()
            .or_else(|| ids.first().map(|id| id.to_string()))
    }

    pub(super) fn scene_snapshot(&self) -> Option<Rc<Snapshot>> {
        let id = self.scene_active_id()?;
        self.scene_ui.cache.get(&self.state, &id)
    }

    /// The value of the property the gesture shows now: the open gesture's candidate, else the
    /// document. Reads through `scene.get`'s function, so it does not depend on the selection.
    pub(super) fn scene_property_now(&self, key: &SliderKey) -> Option<PropertyValue> {
        let attached = self.state.scenes.get(&key.scene)?;
        let reference = PropertyRef::parse(&key.path).ok()?;
        let scene = self
            .state
            .scene_interaction
            .gesture()
            .filter(|open| open.scene() == key.scene)
            .map_or(&attached.scene, |open| open.preview_scene());
        read_property(scene, &reference).ok()
    }

    // ---- writes: always a `Command::Scene` through `execute` ----

    /// Runs a scene command as a gesture or a CLI request and keeps its typed error on screen.
    pub(super) fn scene_execute(
        &mut self,
        command: SceneCommand,
        source: Source,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        let result = self.execute(Command::Scene(command), source, cx);
        self.scene_ui.notice = result.as_ref().err().map(notice_of);
        result
    }

    /// Shows an error that never reached a command (a path that does not resolve).
    pub(super) fn scene_report(&mut self, error: CommandError, cx: &mut Context<Self>) {
        self.scene_ui.notice = Some(notice_of(&error));
        self.activity = Some(super::Activity {
            source: Source::Gesture,
            text: error.to_string(),
            error: true,
        });
        cx.notify();
    }

    /// `scene.select` with `replace` for one entity, in the active scene.
    pub(super) fn scene_select_entity(
        &mut self,
        path: &str,
        source: Source,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        let scene = self.scene_active_id().ok_or_else(no_scene)?;
        self.scene_execute(
            SceneCommand::Select {
                scene: Some(scene),
                entities: vec![path.to_string()],
                mode: SelectMode::Replace,
            },
            source,
            cx,
        )
    }

    /// Resets one property to the catalog default: `scene.edit` with a `reset` op, one undo entry.
    pub(super) fn scene_reset_property(
        &mut self,
        path: &str,
        source: Source,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        let scene = self.scene_active_id().ok_or_else(no_scene)?;
        let reference = PropertyRef::parse(path)?;
        self.scene_execute(
            SceneCommand::Edit {
                scene,
                ops: vec![EditOp::reset(reference)],
                dry_run: false,
            },
            source,
            cx,
        )
    }

    /// Cancels the gesture the core holds on the active scene, whoever began it. The panel's
    /// button and the `scene.panel` action; Escape reaches only the slider the pointer owns.
    pub(super) fn scene_cancel_open_gesture(
        &mut self,
        source: Source,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        let id = self
            .state
            .scene_interaction
            .gesture()
            .map(|open| open.id().to_string())
            .ok_or_else(|| CommandError::new("no_gesture", "No scene gesture is open"))?;
        self.scene_ui.gesture.clear();
        self.scene_execute(
            SceneCommand::Gesture(GestureCommand::Cancel { gesture: id }),
            source,
            cx,
        )
    }

    /// The semantic equivalent of dragging one slider: begin, one update, commit. It is one
    /// gesture and one undo entry, through the same commands the pointer sends.
    pub(super) fn scene_set_component(
        &mut self,
        path: &str,
        index: usize,
        value: f64,
        source: Source,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        let scene = self.scene_active_id().ok_or_else(no_scene)?;
        let reference = PropertyRef::parse(path)?;
        let key = SliderKey {
            scene,
            entity: entity_path(&entity_of_property(&reference)),
            path: path.to_string(),
            index,
        };
        let op = self.scene_edit_op(&key, value)?;
        let gesture = self.scene_gesture_begin(&key, cx)?;
        let update = SceneCommand::Gesture(GestureCommand::Update {
            gesture: gesture.clone(),
            ops: vec![op],
        });
        if let Err(error) = self.scene_execute(update, source, cx) {
            let _ = self.scene_cancel_open_gesture(source, cx);
            self.scene_ui.notice = Some(notice_of(&error));
            return Err(error);
        }
        self.scene_execute(
            SceneCommand::Gesture(GestureCommand::Commit { gesture }),
            source,
            cx,
        )
    }

    pub(super) fn scene_set_mode(&mut self, mode: InspectorMode, cx: &mut Context<Self>) {
        if self.scene_ui.mode == mode {
            return;
        }
        self.scene_ui.mode = mode;
        cx.notify();
    }

    pub(super) fn scene_choose(
        &mut self,
        scene: &str,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        if self.state.scenes.get(scene).is_none() {
            return Err(CommandError::unknown_element("scene", scene));
        }
        self.scene_ui.chosen = Some(scene.to_string());
        cx.notify();
        Ok(json!({ "target": "scene.picker", "field": "value", "value": scene, "changed": true }))
    }

    /// The Kit tree's own selection moved (arrow keys, a click): follow it into the core.
    fn scene_tree_moved(&mut self, tree: &Entity<TreeState>, cx: &mut Context<Self>) {
        let Some(id) = tree
            .read(cx)
            .selected_item()
            .map(|item| item.id.to_string())
        else {
            return;
        };
        if self.scene_ui.pushed_selection.as_deref() == Some(id.as_str()) {
            return;
        }
        self.scene_ui.pushed_selection = Some(id.clone());
        let _ = self.scene_select_entity(&id, Source::Gesture, cx);
    }

    /// The project was replaced: nothing the panel tracked belongs to the new one.
    pub(super) fn scene_project_changed(&mut self) {
        self.scene_ui.gesture.clear();
        self.scene_ui.sliders.clear();
        self.scene_ui.fx.clear();
        self.scene_ui.chosen = None;
        self.scene_ui.notice = None;
        self.scene_ui.pushed_selection = None;
        self.scene_ui.expansion.clear();
        self.scene_ui.tree_signature.clear();
    }

    // ---- sync of Kit controls with the model (called from painting) ----

    /// Pushes the scene picker's choices and the tree into their Kit states when they changed.
    pub(super) fn sync_scene_controls(
        &mut self,
        snapshot: Option<&Snapshot>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let choices: Vec<(String, String)> = self
            .state
            .scenes
            .iter()
            .map(|entry| (entry.id.clone(), format!("{} ({})", entry.name, entry.id)))
            .collect();
        if choices != self.scene_ui.picker_choices {
            let labels: Vec<String> = choices.iter().map(|(_, label)| label.clone()).collect();
            self.scene_ui.picker.update(cx, |select, cx| {
                select.set_items(SearchableVec::new(labels), window, cx)
            });
            self.scene_ui.picker_choices = choices;
        }
        if let Some(snapshot) = snapshot
            && let Some((_, label)) = self
                .scene_ui
                .picker_choices
                .iter()
                .find(|(id, _)| *id == snapshot.scene)
        {
            let label = label.clone();
            if self.scene_ui.picker.read(cx).selected_value() != Some(&label) {
                self.scene_ui.picker.update(cx, |select, cx| {
                    select.set_selected_value(&label, window, cx)
                });
            }
        }
        self.sync_scene_tree(snapshot, cx);
    }

    fn sync_scene_tree(&mut self, snapshot: Option<&Snapshot>, cx: &mut Context<Self>) {
        let mut signature: Vec<(String, bool)> = Vec::new();
        if let Some(snapshot) = snapshot {
            signature.push((format!("scene:{}", snapshot.scene), true));
            let mut visible = Vec::new();
            TreeEntry::visible(
                &snapshot.entries,
                &|path| {
                    self.scene_ui
                        .is_expanded(path, depth_of(&snapshot.entries, path))
                },
                0,
                &mut visible,
            );
            signature.extend(
                visible
                    .iter()
                    .map(|(_, entry)| (entry.path.clone(), entry.selected)),
            );
        }
        if signature == self.scene_ui.tree_signature {
            return;
        }
        let items: Vec<_> = snapshot
            .map(|snapshot| {
                snapshot
                    .entries
                    .iter()
                    .map(|entry| tree_item(&self.scene_ui, entry, 0))
                    .collect()
            })
            .unwrap_or_default();
        let selected = snapshot
            .and_then(|snapshot| snapshot.selection.first().cloned())
            .filter(|_| snapshot.is_some_and(|snapshot| snapshot.selection.len() == 1));
        // Recorded before the push: the Kit notifies from inside it, and `scene_tree_moved` must
        // see the entity it is about to be told about as already known.
        self.scene_ui.pushed_selection = selected.clone();
        self.scene_ui.tree.update(cx, |tree, cx| {
            tree.set_items(items, cx);
            let row = selected
                .as_ref()
                .and_then(|path| tree.index_of(&SharedString::from(path.clone())));
            tree.set_selected_index(row, cx);
        });
        self.scene_ui.tree_signature = signature;
    }

    // ---- automation: semantic nodes ----

    /// Window-only nodes of the scene inspector. They describe what the panel shows, so a client
    /// that cannot see pixels can find every entity, value, disabled reason and the monitor state.
    pub(super) fn scene_nodes(&self) -> Vec<Value> {
        let monitor = self.scene_monitor();
        let mut nodes = vec![
            json!({
                "id": "inspector.mode", "kind": "tabs", "label": "Inspetor",
                "parent": "region:inspector", "children": [], "enabled": true, "visible": true,
                "value": self.scene_ui.mode.id(),
                "fields": [{
                    "field": "value", "label": "Modo",
                    "options": InspectorMode::ALL.map(InspectorMode::id),
                    "value": self.scene_ui.mode.id(),
                }],
                "actions": [],
            }),
            monitor.node(),
        ];
        let snapshot = self.scene_snapshot();
        let mut panel_children: Vec<Value> = Vec::new();
        let scenes: Vec<&str> = self.state.scenes.ids();
        if scenes.len() > 1 {
            panel_children.push(json!("scene.picker"));
            nodes.push(json!({
                "id": "scene.picker", "kind": "select", "label": "Cena ativa",
                "parent": "scene.panel", "children": [], "enabled": true, "visible": true,
                "value": snapshot.as_ref().map(|s| s.scene.clone()),
                "fields": [{ "field": "value", "label": "Cena", "options": scenes,
                    "value": snapshot.as_ref().map(|s| s.scene.clone()) }],
                "actions": [],
            }));
        }
        let Some(snapshot) = snapshot else {
            nodes.push(json!({
                "id": "scene.panel", "kind": "panel", "label": "Inspetor de cena",
                "parent": "region:inspector", "children": panel_children, "enabled": false,
                "disabled_reason": NO_SCENE_REASON, "visible": self.scene_ui.mode == InspectorMode::Scene,
                "fields": [], "actions": [],
                "state": {
                    "mode": self.scene_ui.mode.id(), "scene": null, "scenes": [],
                    "available": false, "reason": NO_SCENE_REASON,
                    "monitor": monitor.report(),
                },
            }));
            return nodes;
        };

        panel_children.push(json!("scene.tree"));
        panel_children.push(json!(scene_fx::SECTION_ID));
        nodes.extend(self.fx_nodes());
        let mut visible = Vec::new();
        TreeEntry::visible(
            &snapshot.entries,
            &|path| {
                self.scene_ui
                    .is_expanded(path, depth_of(&snapshot.entries, path))
            },
            0,
            &mut visible,
        );
        let listed: Vec<&(usize, &TreeEntry)> = visible.iter().take(MAX_LISTED_ENTITIES).collect();
        let listed_paths: Vec<&str> = listed
            .iter()
            .map(|(_, entry)| entry.path.as_str())
            .collect();
        let top: Vec<String> = snapshot
            .entries
            .iter()
            .filter(|entry| listed_paths.contains(&entry.path.as_str()))
            .map(|entry| entity_node_id(&entry.path))
            .collect();
        nodes.push(json!({
            "id": "scene.tree", "kind": "tree", "label": "Árvore da cena",
            "parent": "scene.panel", "children": top, "enabled": true, "visible": true,
            "fields": [], "actions": [],
            "state": {
                "scene": snapshot.scene, "visible_entities": visible.len(),
                "listed": listed.len(), "truncated": visible.len() > listed.len(),
                "selection": snapshot.selection,
            },
        }));
        for (depth, entry) in listed.iter().map(|pair| (pair.0, pair.1)) {
            let expanded = self.scene_ui.is_expanded(&entry.path, depth);
            let children: Vec<String> = entry
                .children
                .iter()
                .filter(|child| listed_paths.contains(&child.path.as_str()))
                .map(|child| entity_node_id(&child.path))
                .collect();
            let parent = visible
                .iter()
                .find(|(_, candidate)| {
                    candidate
                        .children
                        .iter()
                        .any(|child| child.path == entry.path)
                })
                .map_or_else(
                    || "scene.tree".to_string(),
                    |(_, parent)| entity_node_id(&parent.path),
                );
            let mut actions = vec![json!({ "action": "select", "label": "Selecionar" })];
            if entry.children_count > 0 {
                let action = if expanded { "collapse" } else { "expand" };
                actions.push(json!({ "action": action, "label": if expanded { "Recolher" } else { "Expandir" } }));
            }
            nodes.push(json!({
                "id": entity_node_id(&entry.path), "kind": "treeitem", "label": entry.label(),
                "parent": parent, "children": children, "enabled": true, "visible": true,
                "fields": [], "actions": actions,
                "state": {
                    "path": entry.path, "entity_kind": entry.kind, "id": entry.id,
                    "node_kind": entry.node_kind, "depth": depth, "selected": entry.selected,
                    "expanded": expanded, "children_count": entry.children_count,
                    "editable": entry.editable,
                },
            }));
        }

        let target_state = match &snapshot.target {
            Target::None => json!({ "kind": "none",
                "reason": "Selecione uma entidade na árvore para editar suas propriedades" }),
            Target::Multiple(count) => json!({ "kind": "multiple", "count": count,
                "reason": format!("{count} entidades selecionadas; edite uma por vez") }),
            Target::Entity(target) => {
                for property in &target.properties {
                    panel_children.push(json!(property_node_id(&property.path)));
                    nodes.push(self.property_node(
                        &snapshot.scene,
                        &target.path,
                        property,
                        snapshot.gesture_open(),
                    ));
                }
                json!({ "kind": "entity", "path": target.path,
                    "entity_kind": target.kind, "label": target.label,
                    "has_catalog_entries": !target.properties.is_empty() })
            }
        };
        let gesture = snapshot.gesture.as_ref().map(|gesture| {
            json!({
                "id": gesture.id, "updates": gesture.updates, "has_candidate": gesture.has_candidate,
                "invalidated": gesture.invalidated,
            })
        });
        nodes.push(json!({
            "id": "scene.panel", "kind": "panel", "label": "Inspetor de cena",
            "parent": "region:inspector", "children": panel_children, "enabled": true,
            "visible": self.scene_ui.mode == InspectorMode::Scene, "fields": [],
            "actions": [{ "action": "cancel_gesture", "label": "Cancelar gesto",
                "enabled": snapshot.gesture.is_some() }],
            "state": {
                "mode": self.scene_ui.mode.id(), "scene": snapshot.scene,
                "scene_name": snapshot.scene_name, "scenes": scenes_ids(&self.state),
                "available": true, "selection": snapshot.selection, "target": target_state,
                "gesture": gesture,
                "notice": self.scene_ui.notice.as_ref().map(|n| json!({ "code": n.code, "message": n.message })),
                "monitor": monitor.report(),
            },
        }));
        nodes
    }

    /// The range the slider of `key` has, or will get on the next paint: the window it was built
    /// with while the value is inside it, else a fresh one around the value. The semantic node
    /// reports this, so a client sees the limits the pointer actually has.
    fn scene_slider_window(&self, key: &SliderKey, spec: &PropertySpec, value: f64) -> Scrub {
        self.scene_ui
            .sliders
            .get(key)
            .map(|entry| entry.scrub)
            .filter(|window| (window.min..=window.max).contains(&value))
            .unwrap_or_else(|| scrub(spec, key.index, value))
    }

    fn property_node(
        &self,
        scene: &str,
        entity: &str,
        property: &PropertyView,
        gesture_open: bool,
    ) -> Value {
        let spec = property.spec;
        let shown = property.shown();
        let values = shown.as_ref().map(component_values).unwrap_or_default();
        let fields: Vec<Value> = components(spec.kind)
            .iter()
            .enumerate()
            .map(|(index, component)| {
                let value = values.get(index).copied();
                let key = SliderKey {
                    scene: scene.to_string(),
                    entity: entity.to_string(),
                    path: property.path.clone(),
                    index,
                };
                let window = self.scene_slider_window(&key, spec, value.unwrap_or(0.0));
                json!({
                    "field": component.field, "label": format!("{} {}", property.label, component.label).trim(),
                    "type": "number", "value": value,
                    "min": window.min, "max": window.max, "step": window.step,
                    "unit": unit_label(spec.unit.as_str()),
                })
            })
            .collect();
        let reset = property.reset_state(gesture_open);
        let (enabled, reason) = match &property.availability {
            Availability::Editable => (true, None),
            Availability::Unavailable(reason) => (false, Some(reason.clone())),
        };
        let mut node = json!({
            "id": property_node_id(&property.path), "kind": "property", "label": property.label,
            "parent": "scene.panel", "children": [], "enabled": enabled, "visible": true,
            "fields": if enabled { fields } else { Vec::new() },
            "actions": [{
                "action": "reset", "label": format!("Redefinir {}", property.label),
                "enabled": reset.is_ok(), "disabled_reason": reset.as_ref().err(),
            }],
            "value": shown.map(|value| value.to_json()),
            "state": {
                "path": property.path, "name": spec.name, "kind": spec.kind.as_str(),
                "unit": spec.unit.as_str(),
                "live": property.live.map(|value| value.to_json()),
                "candidate": property.candidate.map(|value| value.to_json()),
                "default": spec.default.map(|value| value.to_json()),
                "modified": property.is_modified(),
                "applies_to": spec.applies_to.describe(),
            },
        });
        if let Some(reason) = reason {
            node["disabled_reason"] = json!(reason);
        }
        node
    }

    pub(super) fn scene_node(&self, id: &str) -> Option<Value> {
        self.scene_nodes().into_iter().find(|node| node["id"] == id)
    }

    /// Children the scene inspector adds to nodes of the core's tree.
    pub(super) fn adopt_scene(&self, node: &mut Value) {
        let extra = match node["id"].as_str() {
            Some("region:inspector") => ["inspector.mode", "scene.panel"].as_slice(),
            Some("region:monitor") => ["monitor.scene"].as_slice(),
            _ => return,
        };
        if let Some(children) = node["children"].as_array_mut() {
            children.extend(extra.iter().map(|id| json!(id)));
        }
    }

    /// `invoke` on a scene node. `None` when the target is not one, so the caller falls through
    /// to the core's tree.
    pub fn scene_invoke(
        &mut self,
        target: &str,
        action: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Option<Result<Value, CommandError>> {
        if let Some(outcome) = self.fx_invoke(target, action, cx) {
            return Some(outcome);
        }
        if let Some(path) = target.strip_prefix(ENTITY_PREFIX) {
            return Some(self.invoke_entity(target, path, action, cx));
        }
        if let Some(path) = target.strip_prefix(PROPERTY_PREFIX) {
            return Some(self.invoke_property(target, path, action, cx));
        }
        match target {
            "scene.panel" => Some(match action {
                Some("cancel_gesture") | None => self.scene_cancel_open_gesture(Source::Cli, cx),
                Some(other) => Err(unknown_action(target, other, &["cancel_gesture"])),
            }),
            "monitor.scene" if !self.scene_monitor().is_available() => {
                Some(Err(self.scene_monitor().show_error().target(target)))
            }
            "monitor.scene" => Some(match self.scene_active_id() {
                Some(scene) => self.show_directed_scene(&scene, cx),
                None => Err(self.scene_monitor().show_error().target(target)),
            }),
            _ => None,
        }
    }

    fn invoke_entity(
        &mut self,
        target: &str,
        path: &str,
        action: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        match action.unwrap_or("select") {
            "select" => self.scene_select_entity(path, Source::Cli, cx),
            toggle @ ("expand" | "collapse") => {
                let snapshot = self.scene_snapshot().ok_or_else(no_scene)?;
                if TreeEntry::find(&snapshot.entries, path).is_none() {
                    return Err(CommandError::unknown_element("scene entity", path));
                }
                let expanded = toggle == "expand";
                self.scene_ui.expansion.insert(path.to_string(), expanded);
                cx.notify();
                Ok(
                    json!({ "target": target, "action": toggle, "expanded": expanded, "changed": true }),
                )
            }
            other => Err(unknown_action(
                target,
                other,
                &["select", "expand", "collapse"],
            )),
        }
    }

    fn invoke_property(
        &mut self,
        target: &str,
        path: &str,
        action: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        match action.unwrap_or("reset") {
            "reset" => {
                let snapshot = self.scene_snapshot().ok_or_else(no_scene)?;
                let property = snapshot
                    .property(path)
                    .ok_or_else(|| CommandError::unknown_element("scene property", path))?;
                if let Err(reason) = property.reset_state(snapshot.gesture_open()) {
                    return Err(CommandError::disabled(reason).target(target));
                }
                self.scene_reset_property(path, Source::Cli, cx)
            }
            other => Err(unknown_action(target, other, &["reset"])),
        }
    }

    /// `set` on a scene node: the inspector mode, the active scene, or one component of a
    /// property (one gesture, one undo entry). `None` when the target is not one of these.
    pub fn scene_set(
        &mut self,
        target: &str,
        field: &str,
        raw: &Raw,
        cx: &mut Context<Self>,
    ) -> Option<Result<Value, CommandError>> {
        let value_field = |field: &str| -> Result<(), CommandError> {
            if field == "value" {
                return Ok(());
            }
            Err(CommandError::new(
                "unknown_field",
                format!("`{target}` has no settable field `{field}`"),
            )
            .target(target))
        };
        match target {
            "inspector.mode" => {
                return Some(value_field(field).and_then(|()| {
                    let text = match raw {
                        Raw::Text(text) => text.as_str(),
                        _ => "",
                    };
                    let mode = InspectorMode::parse(text).ok_or_else(|| {
                        CommandError::invalid_value(
                            "Expected `clip` or `scene`",
                            json!(InspectorMode::ALL.map(InspectorMode::id)),
                        )
                    })?;
                    self.scene_set_mode(mode, cx);
                    Ok(json!({ "target": target, "field": "value", "value": mode.id(), "changed": true }))
                }));
            }
            "scene.picker" => {
                return Some(value_field(field).and_then(|()| match raw {
                    Raw::Text(scene) => self.scene_choose(scene, cx),
                    _ => Err(CommandError::invalid_value(
                        "Expected a scene ID",
                        json!("string"),
                    )),
                }));
            }
            _ => {}
        }
        if let Some(outcome) = self.fx_set(target, field, raw, cx) {
            return Some(outcome);
        }
        let path = target.strip_prefix(PROPERTY_PREFIX)?;
        Some(self.set_property_component(target, path, field, raw, cx))
    }

    fn set_property_component(
        &mut self,
        target: &str,
        path: &str,
        field: &str,
        raw: &Raw,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        let snapshot = self.scene_snapshot().ok_or_else(no_scene)?;
        let property = snapshot
            .property(path)
            .ok_or_else(|| CommandError::unknown_element("scene property", path))?;
        if let Availability::Unavailable(reason) = &property.availability {
            return Err(CommandError::disabled(reason.clone()).target(target));
        }
        let declared = components(property.spec.kind);
        let Some(index) = declared
            .iter()
            .position(|component| component.field == field)
        else {
            return Err(CommandError::new(
                "unknown_field",
                format!("`{target}` has no settable field `{field}`"),
            )
            .target(target)
            .details(json!({ "fields": declared.iter().map(|c| c.field).collect::<Vec<_>>() })));
        };
        let value = raw.number()?;
        self.scene_set_component(path, index, value, Source::Cli, cx)
    }
}

const ENTITY_PREFIX: &str = "scene.entity:";
const PROPERTY_PREFIX: &str = "scene.property:";

pub const NO_SCENE_REASON: &str = "Nenhuma cena dirigida está anexada. Use Anexar cena na janela ou scene.attach (ui op ou headless)";

fn no_scene() -> CommandError {
    CommandError::new("unknown_element", NO_SCENE_REASON)
}

fn entity_node_id(path: &str) -> String {
    format!("{ENTITY_PREFIX}{path}")
}

fn property_node_id(path: &str) -> String {
    format!("{PROPERTY_PREFIX}{path}")
}

fn scenes_ids(state: &crate::state::EditorState) -> Vec<String> {
    state.scenes.ids().into_iter().map(str::to_string).collect()
}

fn unknown_action(target: &str, action: &str, available: &[&str]) -> CommandError {
    CommandError::new(
        "unknown_action",
        format!("`{target}` has no action `{action}`"),
    )
    .target(target)
    .details(json!({ "actions": available }))
}

/// Depth of `path` in the tree: the number of ancestors it has. Needed because the default
/// expansion depends on it, and the visible-walk callback only sees the path.
pub(super) fn depth_of(entries: &[TreeEntry], path: &str) -> usize {
    fn walk(entries: &[TreeEntry], path: &str, depth: usize) -> Option<usize> {
        for entry in entries {
            if entry.path == path {
                return Some(depth);
            }
            if let Some(found) = walk(&entry.children, path, depth + 1) {
                return Some(found);
            }
        }
        None
    }
    walk(entries, path, 0).unwrap_or(0)
}
