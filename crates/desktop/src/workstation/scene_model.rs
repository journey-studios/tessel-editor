//! The scene inspector's view model (BL014/BL021, `docs/features/scene-inspector.md`).
//!
//! Pure data, no GPUI: it reads an attached scene through the public functions that the
//! `scene.tree`, `scene.selection`, `scene.catalog` and `scene.get` commands themselves call
//! (`tree_json`, `Selection`, `catalog`, `read_property`), so the tree, the selection and the
//! values the panel shows are the ones a socket or headless client gets. Every write goes
//! through `Command::Scene` (see `scene_ui.rs`); this file never mutates the project.
//!
//! WHY reads call library functions instead of running `scene.*` commands: reads happen while
//! painting and while answering the automation tree, both of which only hold `&self`.
//! `EditorState::execute` takes `&mut self` (and `scene.selection` prunes), so running the
//! commands there would force a mutable borrow inside `render`.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::Value;
use tessel_editor_core::directed_scene::{DirectedScene, Quat, Vec3};
use tessel_editor_core::scene_edit::{
    Applicability, Limit, PropertyRef, PropertySpec, PropertyValue, ValueKind, catalog,
    read_property,
};
use tessel_editor_core::scene_interaction::{Invalidation, entity_path, exists, tree_json};

use crate::state::EditorState;

/// Entities of the tree that a flat consumer (the automation tree) lists. WHY a cap: a bundle
/// may hold 100 000 nodes, and every node of the automation tree is a JSON object per request.
pub const MAX_LISTED_ENTITIES: usize = 256;

// ---- tree ----

/// One entity of the `scene.tree` answer.
#[derive(Debug, Clone, PartialEq)]
pub struct TreeEntry {
    pub path: String,
    pub kind: String,
    pub id: String,
    pub node_kind: Option<String>,
    pub selected: bool,
    pub children_count: usize,
    /// Catalog property paths `scene.edit` accepts for this entity.
    pub editable: Vec<String>,
    pub children: Vec<TreeEntry>,
}

impl TreeEntry {
    /// Reads one entry of the `scene.tree` JSON. `None` only when the shape is not the one the
    /// core documents (a new core with a different tree must not crash the window).
    pub fn from_json(value: &Value) -> Option<Self> {
        let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_string);
        let children = value
            .get("children")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Self::from_json).collect())
            .unwrap_or_default();
        Some(Self {
            path: text("path")?,
            kind: text("kind")?,
            id: text("id")?,
            node_kind: text("node_kind"),
            selected: value.get("selected").and_then(Value::as_bool) == Some(true),
            children_count: value
                .get("children_count")
                .and_then(Value::as_u64)
                .map_or(0, |count| count as usize),
            editable: value
                .get("editable")
                .and_then(Value::as_array)
                .map(|paths| {
                    paths
                        .iter()
                        .filter_map(|path| path.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            children,
        })
    }

    /// The top-level entries of a whole-bundle `scene.tree` answer (the bundle itself is not an
    /// entity a user selects).
    pub fn top_level(answer: &Value) -> Vec<Self> {
        answer
            .get("children")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Self::from_json).collect())
            .unwrap_or_default()
    }

    pub fn label(&self) -> String {
        match (self.kind.as_str(), &self.node_kind) {
            ("node", Some(kind)) => format!("{} · {kind}", self.id),
            ("node", None) => self.id.clone(),
            ("scene_definition", _) => format!("Cena {}", self.id),
            ("scene_instance", _) => format!("Instância {}", self.id),
            ("performance_track", _) => format!("Faixa {}", self.id),
            ("camera_clip", _) => format!("Câmera {}", self.id),
            ("shot", _) => format!("Plano {}", self.id),
            ("asset", _) => format!("Asset {}", self.id),
            ("composition_pass", _) => format!("Passe {}", self.id),
            _ => self.id.clone(),
        }
    }

    pub fn find<'a>(entries: &'a [Self], path: &str) -> Option<&'a Self> {
        for entry in entries {
            if entry.path == path {
                return Some(entry);
            }
            if let Some(found) = Self::find(&entry.children, path) {
                return Some(found);
            }
        }
        None
    }

    /// Pre-order walk of the entries a user can see: a collapsed entry hides its children.
    /// `expanded` is the adapter's expansion state.
    pub fn visible<'a>(
        entries: &'a [Self],
        expanded: &dyn Fn(&str) -> bool,
        depth: usize,
        out: &mut Vec<(usize, &'a Self)>,
    ) {
        for entry in entries {
            out.push((depth, entry));
            if expanded(&entry.path) {
                Self::visible(&entry.children, expanded, depth + 1, out);
            }
        }
    }
}

// ---- properties ----

/// Why a control cannot act, or `Editable`. The text is shown to the user and exposed as the
/// `disabled_reason` of the automation node.
#[derive(Debug, Clone, PartialEq)]
pub enum Availability {
    Editable,
    Unavailable(String),
}

/// One catalog property of the selected entity.
#[derive(Debug, Clone)]
pub struct PropertyView {
    pub path: String,
    pub spec: &'static PropertySpec,
    pub label: &'static str,
    pub availability: Availability,
    /// The value in the document.
    pub live: Option<PropertyValue>,
    /// The value in the open gesture's candidate, when the gesture has one for this scene.
    pub candidate: Option<PropertyValue>,
}

/// pt-BR names of the catalog entries. A property the catalog gains later falls back to its
/// catalog name, so a new entry is editable before it is translated.
fn label_of(spec: &'static PropertySpec) -> &'static str {
    match spec.name {
        "position" => "Posição",
        "rotation" => "Rotação",
        "scale" => "Escala",
        "width" => "Largura",
        "height" => "Altura",
        "start/position" => "Posição inicial",
        "start/target" => "Alvo inicial",
        "start/fov" => "Campo de visão",
        other => other,
    }
}

impl PropertyView {
    /// What a control shows: the candidate while a gesture previews, otherwise the document.
    pub fn shown(&self) -> Option<PropertyValue> {
        self.candidate.or(self.live)
    }

    /// Differs from the catalog default (a property without a default is never "modified").
    pub fn is_modified(&self) -> bool {
        match (self.shown(), self.spec.default) {
            (Some(shown), Some(default)) => shown != default,
            _ => false,
        }
    }

    pub fn is_editable(&self) -> bool {
        self.availability == Availability::Editable
    }

    /// `Ok` when "reset" can run now; otherwise the reason it cannot, in the user's words.
    pub fn reset_state(&self, gesture_open: bool) -> Result<(), String> {
        if let Availability::Unavailable(reason) = &self.availability {
            return Err(reason.clone());
        }
        let Some(default) = self.spec.default else {
            return Err(format!(
                "O catálogo não define um valor padrão para {}",
                self.label.to_lowercase()
            ));
        };
        if gesture_open {
            return Err("Há um gesto em andamento; solte o controle ou use Escape".to_string());
        }
        if self.live == Some(default) {
            return Err("Já está no valor padrão".to_string());
        }
        Ok(())
    }
}

// ---- values per slider ----

/// One slider of a property: a number, one axis of a vector, or one Euler angle of a rotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Component {
    /// Field name on the automation node (`value`, `x`, `euler_x`).
    pub field: &'static str,
    pub label: &'static str,
}

const NUMBER: &[Component] = &[Component {
    field: "value",
    label: "",
}];
const VECTOR: &[Component] = &[
    Component {
        field: "x",
        label: "X",
    },
    Component {
        field: "y",
        label: "Y",
    },
    Component {
        field: "z",
        label: "Z",
    },
];
// WHY Euler for a quaternion: the catalog stores a unit quaternion, and the importer refuses one
// that is not (`invalid-quaternion-not-normalized`), so a slider per quaternion component could
// only produce rejected edits. The conversion is the core's own (`Quat::from_euler_xyz_degrees`,
// the origin's `eulerDegToQuat`), not a second implementation.
const ROTATION: &[Component] = &[
    Component {
        field: "euler_x",
        label: "X°",
    },
    Component {
        field: "euler_y",
        label: "Y°",
    },
    Component {
        field: "euler_z",
        label: "Z°",
    },
];

pub fn components(kind: ValueKind) -> &'static [Component] {
    match kind {
        ValueKind::Number => NUMBER,
        ValueKind::Vec3 => VECTOR,
        ValueKind::Quaternion => ROTATION,
    }
}

/// The numbers the sliders of `value` show, in `components` order.
pub fn component_values(value: &PropertyValue) -> Vec<f64> {
    match value {
        PropertyValue::Number(number) => vec![*number],
        PropertyValue::Vec3(v) => vec![v.x, v.y, v.z],
        PropertyValue::Quaternion(q) => {
            let euler = q.to_euler_xyz_degrees();
            vec![euler.x, euler.y, euler.z]
        }
    }
}

/// `value` with its `index`-th component replaced. A rotation is rebuilt from the three Euler
/// angles, so the other two keep the angles the panel shows.
pub fn with_component(value: &PropertyValue, index: usize, new: f64) -> Option<PropertyValue> {
    let mut parts = component_values(value);
    *parts.get_mut(index)? = new;
    Some(match value {
        PropertyValue::Number(_) => PropertyValue::Number(new),
        PropertyValue::Vec3(_) => PropertyValue::Vec3(Vec3 {
            x: parts[0],
            y: parts[1],
            z: parts[2],
        }),
        PropertyValue::Quaternion(_) => {
            PropertyValue::Quaternion(Quat::from_euler_xyz_degrees(Vec3 {
                x: parts[0],
                y: parts[1],
                z: parts[2],
            }))
        }
    })
}

// ---- slider range ----

/// What a slider offers for one component. The catalog bounds a few numbers (`0 < width`,
/// `0 < fov < 180`) and nothing else; a slider needs both ends, so unbounded values get a
/// window around the value the control opens with. It is presentation only: the core still
/// decides what is valid, and a value it refuses comes back as a typed error.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scrub {
    pub min: f64,
    pub max: f64,
    pub step: f64,
}

/// 1-2-5 step that gives about 200–500 positions across `span`.
fn nice_step(span: f64) -> f64 {
    let raw = (span / 400.0).max(0.001);
    let magnitude = 10f64.powf(raw.log10().floor());
    let normalized = raw / magnitude;
    let factor = if normalized <= 1.0 {
        1.0
    } else if normalized <= 2.0 {
        2.0
    } else if normalized <= 5.0 {
        5.0
    } else {
        10.0
    };
    factor * magnitude
}

fn lower_edge(limit: Option<Limit>, step: f64) -> Option<f64> {
    limit.map(|limit| {
        if limit.inclusive {
            limit.value
        } else {
            limit.value + step
        }
    })
}

fn upper_edge(limit: Option<Limit>, step: f64) -> Option<f64> {
    limit.map(|limit| {
        if limit.inclusive {
            limit.value
        } else {
            limit.value - step
        }
    })
}

pub fn scrub(spec: &PropertySpec, index: usize, value: f64) -> Scrub {
    if spec.kind == ValueKind::Quaternion {
        // `Euler.setFromQuaternion(.., "XYZ")`: the middle angle is an `asin`, so it stays in ±90.
        let limit = if index == 1 { 90.0 } else { 180.0 };
        return Scrub {
            min: -limit,
            max: limit,
            step: 0.5,
        };
    }
    let range = spec.range;
    let (low, high) = (
        range.and_then(|range| range.min),
        range.and_then(|range| range.max),
    );
    if let (Some(low), Some(high)) = (low, high) {
        let step = nice_step(high.value - low.value);
        let min = lower_edge(Some(low), step).unwrap_or(low.value);
        let max = upper_edge(Some(high), step).unwrap_or(high.value);
        return Scrub { min, max, step };
    }
    let half = (value.abs() * 2.0).max(5.0);
    let step = nice_step(half * 2.0);
    // Aligned to the step grid so the keyboard's `quantize` does not move a value it did not step.
    let mut min = ((value - half) / step).floor() * step;
    let mut max = ((value + half) / step).ceil() * step;
    if let Some(edge) = lower_edge(low, step) {
        min = min.max(edge.min(value));
    }
    if let Some(edge) = upper_edge(high, step) {
        max = max.min(edge.max(value));
    }
    Scrub { min, max, step }
}

// ---- text ----

/// Up to three decimals, without trailing zeros (`1.5`, `-2`, `0.125`).
pub fn format_number(value: f64) -> String {
    let text = format!("{value:.3}");
    let trimmed = text.trim_end_matches('0').trim_end_matches('.');
    match trimmed {
        "-0" | "" => "0".to_string(),
        other => other.to_string(),
    }
}

pub fn unit_label(unit: &str) -> &'static str {
    match unit {
        "scene-units" => "un",
        "degrees" => "°",
        _ => "",
    }
}

// ---- snapshot ----

/// An open gesture of the core, as the panel needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct GestureView {
    pub id: String,
    pub scene: String,
    pub updates: u64,
    pub has_candidate: bool,
    /// Why the core would refuse to apply it (`scene_changed`, `project_revision`, ...).
    pub invalidated: Option<&'static str>,
}

#[derive(Debug, Clone)]
pub struct TargetView {
    pub path: String,
    pub kind: String,
    pub label: String,
    pub properties: Vec<PropertyView>,
}

#[derive(Debug, Clone)]
pub enum Target {
    /// Nothing of this scene is selected.
    None,
    Multiple(usize),
    /// One entity, whose kind may have no catalog entries (`properties` is empty).
    Entity(TargetView),
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub scene: String,
    pub scene_name: String,
    pub entries: Vec<TreeEntry>,
    /// Addresses of the entities of this scene in the core's selection.
    pub selection: Vec<String>,
    pub target: Target,
    pub gesture: Option<GestureView>,
}

impl Snapshot {
    /// A gesture of this scene is open and still applicable. A reset would invalidate it.
    pub fn gesture_open(&self) -> bool {
        self.gesture
            .as_ref()
            .is_some_and(|gesture| gesture.invalidated.is_none())
    }

    pub fn property(&self, path: &str) -> Option<&PropertyView> {
        let Target::Entity(target) = &self.target else {
            return None;
        };
        target
            .properties
            .iter()
            .find(|property| property.path == path)
    }
}

fn read_value(
    scene: &tessel_editor_core::directed_scene::DirectedScene,
    path: &str,
) -> Option<PropertyValue> {
    let reference = PropertyRef::parse(path).ok()?;
    read_property(scene, &reference).ok()
}

fn is_camera_entity(kind: &str) -> bool {
    kind == "camera_clip"
}

fn is_node_entity(kind: &str) -> bool {
    kind == "node"
}

fn target_view(
    state: &EditorState,
    scene_id: &str,
    entry: &TreeEntry,
    gesture: Option<&GestureView>,
) -> TargetView {
    let attached = state.scenes.get(scene_id);
    let candidate_scene = state
        .scene_interaction
        .gesture()
        .filter(|open| open.scene() == scene_id && open.has_candidate())
        .filter(|_| gesture.is_some_and(|gesture| gesture.invalidated.is_none()))
        .map(|open| open.preview_scene().clone());
    let properties = catalog()
        .iter()
        .filter(|spec| match spec.applies_to {
            Applicability::Node(_) => is_node_entity(&entry.kind),
            Applicability::CameraClip => is_camera_entity(&entry.kind),
        })
        .map(|spec| {
            let path = format!("{}/{}", entry.path, spec.name);
            let editable = entry.editable.contains(&path);
            let availability = if editable {
                Availability::Editable
            } else {
                Availability::Unavailable(format!(
                    "Só se aplica a {}; esta entidade é {}",
                    spec.applies_to.describe(),
                    entry.node_kind.as_deref().unwrap_or(entry.kind.as_str())
                ))
            };
            let live = attached
                .filter(|_| editable)
                .and_then(|attached| read_value(&attached.scene, &path));
            let candidate = candidate_scene
                .as_ref()
                .filter(|_| editable)
                .and_then(|scene| read_value(scene, &path))
                .filter(|candidate| Some(*candidate) != live);
            PropertyView {
                path,
                spec,
                label: label_of(spec),
                availability,
                live,
                candidate,
            }
        })
        .collect();
    TargetView {
        path: entry.path.clone(),
        kind: entry.kind.clone(),
        label: entry.label(),
        properties,
    }
}

/// What a cached snapshot was built from. A frame that changes none of these reuses it, so
/// repainting a 100 000-node bundle does not walk it again.
#[derive(Debug, Clone)]
struct Signature {
    scene: String,
    document: std::sync::Arc<DirectedScene>,
    selection: Vec<String>,
    preview: Option<std::sync::Arc<DirectedScene>>,
    gesture: Option<(String, u64, Option<&'static str>)>,
}

impl PartialEq for Signature {
    fn eq(&self, other: &Self) -> bool {
        self.scene == other.scene
            && std::sync::Arc::ptr_eq(&self.document, &other.document)
            && match (&self.preview, &other.preview) {
                (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
            && self.selection == other.selection
            && self.gesture == other.gesture
    }
}
impl Eq for Signature {}

/// Builds the snapshot of attached scene `scene_id`, or `None` when it is not attached.
fn build(state: &EditorState, scene_id: &str, selection: Vec<String>) -> Option<Snapshot> {
    let attached = state.scenes.get(scene_id)?;
    let selected_entities: Vec<_> = state
        .scene_interaction
        .selection()
        .entities_of(scene_id)
        .filter(|entity| exists(attached.scene.bundle(), entity))
        .collect();
    let answer = tree_json(&attached.scene, None, None, &selected_entities).ok()?;
    let entries = TreeEntry::top_level(&answer);
    let gesture = state
        .scene_interaction
        .gesture()
        .filter(|open| open.scene() == scene_id)
        .map(|open| GestureView {
            id: open.id().to_string(),
            scene: open.scene().to_string(),
            updates: open.updates(),
            has_candidate: open.has_candidate(),
            invalidated: open
                .invalidation(&state.scenes, state.project.revision)
                .map(Invalidation::as_str),
        });
    let target = match selection.as_slice() {
        [] => Target::None,
        [only] => match TreeEntry::find(&entries, only) {
            Some(entry) => Target::Entity(target_view(state, scene_id, entry, gesture.as_ref())),
            None => Target::None,
        },
        many => Target::Multiple(many.len()),
    };
    Some(Snapshot {
        scene: scene_id.to_string(),
        scene_name: attached.name.clone(),
        entries,
        selection,
        target,
        gesture,
    })
}

/// Memo of the last [`Snapshot`], shared by painting and the automation tree.
#[derive(Default)]
pub struct SnapshotCache(RefCell<Option<(Signature, Rc<Snapshot>)>>);

impl SnapshotCache {
    pub fn get(&self, state: &EditorState, scene_id: &str) -> Option<Rc<Snapshot>> {
        let attached = state.scenes.get(scene_id)?;
        let selection: Vec<String> = state
            .scene_interaction
            .selection()
            .entities_of(scene_id)
            .filter(|entity| exists(attached.scene.bundle(), entity))
            .map(entity_path)
            .collect();
        let open = state
            .scene_interaction
            .gesture()
            .filter(|open| open.scene() == scene_id);
        let signature = Signature {
            scene: scene_id.to_string(),
            document: std::sync::Arc::clone(&attached.scene),
            selection: selection.clone(),
            preview: open
                .filter(|open| open.has_candidate())
                .map(|open| std::sync::Arc::clone(open.preview_scene())),
            gesture: open.map(|open| {
                (
                    open.id().to_string(),
                    open.updates(),
                    open.invalidation(&state.scenes, state.project.revision)
                        .map(Invalidation::as_str),
                )
            }),
        };
        if let Some((cached, snapshot)) = self.0.borrow().as_ref()
            && *cached == signature
        {
            return Some(snapshot.clone());
        }
        let snapshot = Rc::new(build(state, scene_id, selection)?);
        *self.0.borrow_mut() = Some((signature, snapshot.clone()));
        Some(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use core::prelude::v1::test;

    use serde_json::json;
    use tessel_editor_core::Command;
    use tessel_editor_core::scene_interaction::SelectMode;
    use tessel_editor_core::scene_project::{SceneCommand, SceneSource};
    use tessel_editor_core::state::ProjectSettings;

    use super::*;

    pub(crate) const BUNDLE: &str = include_str!(
        "../../../../docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json"
    );

    pub(crate) fn state_with_scene() -> EditorState {
        let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
        state
            .execute(Command::Scene(SceneCommand::Attach {
                id: Some("street".into()),
                name: None,
                source: SceneSource::Text(BUNDLE.to_string()),
            }))
            .unwrap();
        state
    }

    fn select(state: &mut EditorState, paths: &[&str], mode: SelectMode) {
        state
            .execute(Command::Scene(SceneCommand::Select {
                scene: Some("street".into()),
                entities: paths.iter().map(|path| path.to_string()).collect(),
                mode,
            }))
            .unwrap();
    }

    #[test]
    fn the_tree_is_the_one_scene_tree_answers() {
        let mut state = state_with_scene();
        select(
            &mut state,
            &["scene/scene-a/node/figure"],
            SelectMode::Replace,
        );
        let answer = state
            .execute(Command::Scene(SceneCommand::Tree {
                scene: "street".into(),
                root: None,
                depth: None,
            }))
            .unwrap();
        let from_command = TreeEntry::top_level(&answer["tree"]);
        let snapshot = SnapshotCache::default().get(&state, "street").unwrap();
        assert_eq!(snapshot.entries, from_command);
        assert!(
            TreeEntry::find(&snapshot.entries, "scene/scene-a/node/figure")
                .unwrap()
                .selected
        );
        assert_eq!(snapshot.selection, vec!["scene/scene-a/node/figure"]);
    }

    #[test]
    fn an_image_plane_lists_five_node_properties_with_the_catalog_values() {
        let mut state = state_with_scene();
        select(
            &mut state,
            &["scene/scene-a/node/figure"],
            SelectMode::Replace,
        );
        let snapshot = SnapshotCache::default().get(&state, "street").unwrap();
        let Target::Entity(target) = &snapshot.target else {
            panic!("one entity is selected");
        };
        let names: Vec<_> = target.properties.iter().map(|p| p.spec.name).collect();
        assert_eq!(names, ["position", "rotation", "scale", "width", "height"]);
        assert!(target.properties.iter().all(PropertyView::is_editable));
        let position = snapshot
            .property("scene/scene-a/node/figure/position")
            .unwrap();
        assert_eq!(
            position.live,
            Some(PropertyValue::Vec3(Vec3 {
                x: 0.5,
                y: 0.0,
                z: 0.0
            }))
        );
        let width = snapshot
            .property("scene/scene-a/node/figure/width")
            .unwrap();
        assert_eq!(width.live, Some(PropertyValue::Number(2.0)));
    }

    #[test]
    fn a_group_node_explains_why_width_and_height_are_unavailable() {
        let mut state = state_with_scene();
        select(
            &mut state,
            &["scene/scene-a/node/root"],
            SelectMode::Replace,
        );
        let snapshot = SnapshotCache::default().get(&state, "street").unwrap();
        let width = snapshot.property("scene/scene-a/node/root/width").unwrap();
        let Availability::Unavailable(reason) = &width.availability else {
            panic!("a group has no width");
        };
        assert!(reason.contains("imagePlane"), "{reason}");
        assert!(width.live.is_none());
        assert!(width.reset_state(false).is_err());
    }

    #[test]
    fn a_camera_clip_lists_the_three_start_properties() {
        let mut state = state_with_scene();
        select(&mut state, &["camera/cam-main"], SelectMode::Replace);
        let snapshot = SnapshotCache::default().get(&state, "street").unwrap();
        let Target::Entity(target) = &snapshot.target else {
            panic!("one entity is selected");
        };
        let names: Vec<_> = target.properties.iter().map(|p| p.spec.name).collect();
        assert_eq!(names, ["start/position", "start/target", "start/fov"]);
        let fov = snapshot.property("camera/cam-main/start/fov").unwrap();
        assert_eq!(fov.live, Some(PropertyValue::Number(45.0)));
        // No default in the catalog: reset is unavailable and says so.
        let reason = fov.reset_state(false).unwrap_err();
        assert!(reason.contains("padrão"), "{reason}");
    }

    #[test]
    fn selection_states_cover_none_many_and_kinds_without_a_catalog() {
        let mut state = state_with_scene();
        let cache = SnapshotCache::default();
        assert!(matches!(
            cache.get(&state, "street").unwrap().target,
            Target::None
        ));
        select(
            &mut state,
            &["scene/scene-a/node/figure", "scene/scene-a/node/root"],
            SelectMode::Replace,
        );
        assert!(matches!(
            cache.get(&state, "street").unwrap().target,
            Target::Multiple(2)
        ));
        select(&mut state, &["shot/shot-1"], SelectMode::Replace);
        let snapshot = cache.get(&state, "street").unwrap();
        let Target::Entity(target) = &snapshot.target else {
            panic!("a shot is one entity");
        };
        assert!(target.properties.is_empty());
    }

    #[test]
    fn a_gesture_candidate_is_shown_without_touching_the_document() {
        let mut state = state_with_scene();
        select(
            &mut state,
            &["scene/scene-a/node/figure"],
            SelectMode::Replace,
        );
        let begun = state
            .execute(Command::Scene(SceneCommand::Gesture(
                tessel_editor_core::scene_interaction::GestureCommand::Begin {
                    scene: "street".into(),
                    targets: vec!["scene/scene-a/node/figure".into()],
                    label: None,
                },
            )))
            .unwrap();
        let gesture = begun["gesture"].as_str().unwrap().to_string();
        let ops = tessel_editor_core::scene_edit::parse_batch(&json!([
            { "set": "scene/scene-a/node/figure/width", "value": 3 }
        ]))
        .unwrap();
        state
            .execute(Command::Scene(SceneCommand::Gesture(
                tessel_editor_core::scene_interaction::GestureCommand::Update { gesture, ops },
            )))
            .unwrap();
        let snapshot = SnapshotCache::default().get(&state, "street").unwrap();
        let width = snapshot
            .property("scene/scene-a/node/figure/width")
            .unwrap();
        assert_eq!(width.live, Some(PropertyValue::Number(2.0)));
        assert_eq!(width.candidate, Some(PropertyValue::Number(3.0)));
        assert_eq!(width.shown(), Some(PropertyValue::Number(3.0)));
        assert!(snapshot.gesture_open());
        // A reset while a gesture previews would invalidate it, so the panel refuses it.
        assert!(width.reset_state(true).is_err());
    }

    #[test]
    fn the_cache_keeps_its_document_identity_alive_until_replaced() {
        let state = state_with_scene();
        let weak = std::sync::Arc::downgrade(&state.scenes.get("street").unwrap().scene);
        let cache = SnapshotCache::default();
        cache.get(&state, "street").unwrap();
        drop(state);
        assert!(
            weak.upgrade().is_some(),
            "cache identity must prevent address reuse"
        );
        drop(cache);
        assert!(
            weak.upgrade().is_none(),
            "releasing the cache releases its document"
        );
    }

    #[test]
    fn the_cache_follows_the_document_and_the_selection() {
        let mut state = state_with_scene();
        let cache = SnapshotCache::default();
        let first = cache.get(&state, "street").unwrap();
        let again = cache.get(&state, "street").unwrap();
        assert!(
            Rc::ptr_eq(&first, &again),
            "an unchanged frame reuses the snapshot"
        );
        select(
            &mut state,
            &["scene/scene-a/node/figure"],
            SelectMode::Replace,
        );
        let selected = cache.get(&state, "street").unwrap();
        assert!(!Rc::ptr_eq(&first, &selected));
        assert!(cache.get(&state, "missing").is_none());
    }

    #[test]
    fn a_rotation_is_edited_as_euler_angles_and_stays_a_unit_quaternion() {
        let identity = PropertyValue::Quaternion(Quat::IDENTITY);
        assert_eq!(component_values(&identity), vec![0.0, 0.0, 0.0]);
        let turned = with_component(&identity, 2, 90.0).unwrap();
        let PropertyValue::Quaternion(quat) = turned else {
            panic!("still a quaternion");
        };
        assert!((quat.length() - 1.0).abs() < 1e-12);
        let angles = component_values(&turned);
        assert!((angles[2] - 90.0).abs() < 1e-9, "{angles:?}");
        assert!(angles[0].abs() < 1e-9 && angles[1].abs() < 1e-9);
        // Changing one angle keeps the others the panel shows.
        let both = with_component(&turned, 0, 30.0).unwrap();
        let angles = component_values(&both);
        assert!(
            (angles[0] - 30.0).abs() < 1e-9 && (angles[2] - 90.0).abs() < 1e-9,
            "{angles:?}"
        );
    }

    #[test]
    fn vectors_and_numbers_replace_one_component() {
        let vector = PropertyValue::Vec3(Vec3 {
            x: 1.0,
            y: 2.0,
            z: 3.0,
        });
        assert_eq!(
            with_component(&vector, 1, 9.0),
            Some(PropertyValue::Vec3(Vec3 {
                x: 1.0,
                y: 9.0,
                z: 3.0
            }))
        );
        assert_eq!(with_component(&vector, 3, 9.0), None);
        assert_eq!(
            with_component(&PropertyValue::Number(1.0), 0, 4.0),
            Some(PropertyValue::Number(4.0))
        );
    }

    fn spec_named(name: &str) -> &'static PropertySpec {
        catalog()
            .iter()
            .copied()
            .find(|spec| spec.name == name)
            .unwrap()
    }

    #[test]
    fn the_scrub_window_respects_the_catalog_bounds() {
        // 0 < fov < 180: both ends are exclusive, so the slider stops one step inside.
        let fov = scrub(spec_named("start/fov"), 0, 45.0);
        assert!(fov.min > 0.0 && fov.max < 180.0, "{fov:?}");
        assert!(fov.min <= 45.0 && 45.0 <= fov.max);
        // 0 < width: no upper bound, a window around the value, never at or below zero.
        let width = scrub(spec_named("width"), 0, 2.0);
        assert!(width.min > 0.0 && width.max >= 7.0, "{width:?}");
        // Unbounded vectors: a window around the value that always contains it.
        let position = scrub(spec_named("position"), 0, 0.5);
        assert!(position.min <= 0.5 && 0.5 <= position.max && position.step > 0.0);
        let far = scrub(spec_named("position"), 0, 1000.0);
        assert!(far.min <= 1000.0 && 1000.0 <= far.max);
        // Rotation: the middle Euler angle cannot leave ±90.
        assert_eq!(scrub(spec_named("rotation"), 1, 0.0).max, 90.0);
        assert_eq!(scrub(spec_named("rotation"), 0, 0.0).max, 180.0);
    }

    #[test]
    fn a_value_just_above_an_exclusive_minimum_stays_inside_its_window() {
        let tiny = scrub(spec_named("width"), 0, 0.0001);
        assert!(tiny.min <= 0.0001 && tiny.min > 0.0, "{tiny:?}");
    }

    #[test]
    fn numbers_print_without_noise() {
        assert_eq!(format_number(1.5), "1.5");
        assert_eq!(format_number(-2.0), "-2");
        assert_eq!(format_number(0.125), "0.125");
        assert_eq!(format_number(-0.0001), "0");
        assert_eq!(unit_label("scene-units"), "un");
    }

    #[test]
    fn labels_are_stable_and_fall_back_to_the_catalog_name() {
        let entry = |kind: &str, id: &str, node_kind: Option<&str>| TreeEntry {
            path: String::new(),
            kind: kind.into(),
            id: id.into(),
            node_kind: node_kind.map(str::to_string),
            selected: false,
            children_count: 0,
            editable: Vec::new(),
            children: Vec::new(),
        };
        assert_eq!(
            entry("node", "figure", Some("imagePlane")).label(),
            "figure · imagePlane"
        );
        assert_eq!(entry("camera_clip", "cam", None).label(), "Câmera cam");
        assert_eq!(entry("unheard_of", "x", None).label(), "x");
    }
}
