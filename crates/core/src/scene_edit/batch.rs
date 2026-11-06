//! Atomic batches: validate every op against the catalog and the scene's IDs, build a candidate,
//! hand it to the importer, and either return the candidate with a summary or an error that
//! says where it failed.
//!
//! WHY the input is `&DirectedScene` and the result a new `DirectedScene`: a failure at any
//! point leaves the caller with exactly what it passed, and there is no second history. The
//! candidate is the unit of an edit; wiring it into `EditorState::transaction` (so a batch is
//! one undo step) is the integration this module leaves to its caller.

use std::collections::HashMap;

use serde_json::Value;

use crate::directed_scene::{DirectedScene, Entity, EntityKind, ImportError, Limits};

use super::camera_keys::{MovementConversion, convert_first_movement};
use super::catalog::{CameraProperty, NodeProperty, PropertySpec};
use super::error::{
    CANDIDATE_REJECTED, DOCUMENT_MISMATCH, EditError, LocalizedIssue, NO_DEFAULT, NOT_APPLICABLE,
    UNKNOWN_CLIP, UNKNOWN_NODE, UNKNOWN_SCENE,
};
use super::op::EditOp;
use super::reference::PropertyRef;
use super::value::PropertyValue;

/// What one op did, with the values as they were when the op ran (earlier ops of the batch
/// included).
#[derive(Debug, Clone, PartialEq)]
pub struct PropertyChange {
    /// Position of the op in the batch.
    pub index: usize,
    pub property: PropertyRef,
    pub before: PropertyValue,
    pub after: PropertyValue,
}

impl PropertyChange {
    /// The op wrote the value the property already had; the document was left untouched.
    pub fn is_noop(&self) -> bool {
        self.before == self.after
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EditSummary {
    /// One entry per op, in batch order.
    pub changes: Vec<PropertyChange>,
    /// Whether the batch left the document with a value other than the one it started with,
    /// judged on what each touched property holds at the end against what it held at the start.
    /// A component and its parent (`position/x`, `position`) are one datum, so they are compared
    /// together: `position/x -> 9` followed by `position -> original` is `false`, and a final
    /// `position/x` that differs after a parent write that restored the vector is `true`.
    ///
    /// When `false` the batch is a no-op: the candidate is the input scene itself (same document,
    /// same spelling of numbers, no keyframes) and `conversions` is empty.
    pub modified: bool,
    /// Movements a write turned into keyframes (a camera start that changed), in batch order.
    /// Empty for every other batch and for a batch that is a net no-op.
    pub conversions: Vec<MovementConversion>,
}

impl EditSummary {
    fn new(
        changes: Vec<PropertyChange>,
        conversions: Vec<MovementConversion>,
        modified: bool,
    ) -> Self {
        Self {
            changes,
            modified,
            conversions: if modified { conversions } else { Vec::new() },
        }
    }

    /// Ops that changed a value.
    pub fn changed_count(&self) -> usize {
        self.changes
            .iter()
            .filter(|change| !change.is_noop())
            .count()
    }
}

/// A successful batch: the new scene and what changed. The input scene is untouched.
///
/// The candidate passed the importer, which is the structural validation of the BB01/BB02
/// subset plus its structural and temporal camera gates. It is not render certification:
/// the importer still defers domains such as lighting, weather and node animation.
#[derive(Debug, Clone, PartialEq)]
pub struct EditOutcome {
    pub candidate: DirectedScene,
    pub summary: EditSummary,
}

/// Where a property lives in the document, resolved against a scene.
struct Located {
    /// JSON Pointer of the node or clip object that holds the property.
    entity: String,
    /// JSON Pointer of the property's own field.
    field: String,
    spec: &'static PropertySpec,
    /// Position in `cameraClips` when the property belongs to a camera clip.
    clip_slot: Option<usize>,
}

/// Resolves IDs to JSON Pointers. IDs cannot change in an edit (the catalog has no property that
/// renames, adds or removes an entity), so one locator built on the input serves every op.
struct Locator<'a> {
    scene: &'a DirectedScene,
    /// Position of each node in its scene's `nodes` array, built on first use per scene.
    node_slots: HashMap<usize, HashMap<&'a str, usize>>,
}

fn mismatch(what: impl Into<String>) -> EditError {
    EditError::new(DOCUMENT_MISMATCH, what)
}

impl<'a> Locator<'a> {
    fn new(scene: &'a DirectedScene) -> Self {
        Self {
            scene,
            node_slots: HashMap::new(),
        }
    }

    /// Resolves the property's target and checks that it applies there.
    /// Order of findings: scene, node or clip missing, then not applicable.
    fn locate(&mut self, property: &PropertyRef) -> Result<Located, EditError> {
        let scene = self.scene;
        let bundle = scene.bundle();
        let spec = property.spec();
        let mut clip_slot = None;
        let entity = match property {
            PropertyRef::Node {
                scene_id, node_id, ..
            } => {
                let Some(scene_index) = bundle
                    .scenes
                    .iter()
                    .position(|graph| graph.id() == scene_id.as_str())
                else {
                    return Err(
                        EditError::new(UNKNOWN_SCENE, format!("no scene `{scene_id}`"))
                            .with_id(scene_id.as_str()),
                    );
                };
                let graph = &bundle.scenes[scene_index];
                let Some(node) = graph.node(node_id) else {
                    return Err(EditError::new(
                        UNKNOWN_NODE,
                        format!("scene `{scene_id}` has no node `{node_id}`"),
                    )
                    .with_id(node_id.as_str()));
                };
                if !spec.applies_to.accepts_node(node.kind) {
                    return Err(EditError::new(
                        NOT_APPLICABLE,
                        format!(
                            "`{}` applies to {}; node `{node_id}` is {}",
                            spec.name,
                            spec.applies_to.describe(),
                            node.kind.as_str()
                        ),
                    )
                    .with_id(node_id.as_str()));
                }
                let slots = self.node_slots.entry(scene_index).or_insert_with(|| {
                    graph
                        .nodes()
                        .iter()
                        .enumerate()
                        .map(|(slot, node)| (node.id.as_str(), slot))
                        .collect()
                });
                let Some(slot) = slots.get(node_id.as_str()).copied() else {
                    return Err(mismatch(format!("node `{node_id}` has no slot")));
                };
                // Node IDs repeat across scenes, so the scene is verified on its own: a node ID
                // match alone would not catch a drift to another scene's node of the same name.
                verify_id(
                    scene.document(),
                    &format!("/sceneDefinitions/{scene_index}"),
                    scene_id,
                )?;
                format!("/sceneDefinitions/{scene_index}/nodes/{slot}")
            }
            PropertyRef::Camera { clip_id, .. } => {
                let Some(clip_index) = bundle
                    .camera_clips
                    .iter()
                    .position(|clip| clip.id == clip_id.as_str())
                else {
                    return Err(EditError::new(
                        UNKNOWN_CLIP,
                        format!("no camera clip `{clip_id}`"),
                    )
                    .with_id(clip_id.as_str()));
                };
                clip_slot = Some(clip_index);
                format!("/cameraClips/{clip_index}")
            }
        };
        let expected = match property {
            PropertyRef::Node { node_id, .. } => node_id,
            PropertyRef::Camera { clip_id, .. } => clip_id,
        };
        verify_id(scene.document(), &entity, expected)?;
        Ok(Located {
            field: format!("{entity}/{}", spec.document_field),
            entity,
            spec,
            clip_slot,
        })
    }
}

/// Checks that the object at `pointer` has the ID the typed view resolved. The importer keeps
/// both in document order, so this holds; it is checked because a pointer built from a position
/// is only as good as that agreement, and a silent drift would edit the wrong entity.
fn verify_id(document: &Value, pointer: &str, expected: &str) -> Result<(), EditError> {
    let found = document
        .pointer(&format!("{pointer}/id"))
        .and_then(Value::as_str);
    if found == Some(expected) {
        return Ok(());
    }
    Err(mismatch(format!(
        "`{pointer}` is not `{expected}` in the document"
    )))
}

fn read_value(document: &Value, located: &Located) -> Result<PropertyValue, EditError> {
    match document.pointer(&located.field) {
        Some(value) => PropertyValue::read_json(located.spec.kind, value),
        // An optional field the bundle leaves out reads as the catalog's default (`renderOrder`).
        None if located.spec.optional => located.spec.default,
        None => None,
    }
    .ok_or_else(|| mismatch(format!("`{}` holds no readable value", located.field)))
}

fn write_value(
    document: &mut Value,
    located: &Located,
    value: &PropertyValue,
) -> Result<(), EditError> {
    let written = match document.pointer_mut(&located.field) {
        Some(slot) => value.write_json(slot),
        None if located.spec.optional => {
            // The field is absent: add it to the node or clip object that owns it.
            let mut slot = Value::Null;
            let owner = document
                .pointer_mut(&located.entity)
                .and_then(Value::as_object_mut);
            match owner {
                Some(owner) if value.write_json(&mut slot) => {
                    owner.insert(located.spec.document_field.to_string(), slot);
                    true
                }
                _ => false,
            }
        }
        None => false,
    };
    if written {
        return Ok(());
    }
    Err(mismatch(format!(
        "`{}` cannot hold the value",
        located.field
    )))
}

/// `pointer` is the entity itself or something inside it (`/nodes/3` contains `/nodes/3/id`, not
/// `/nodes/30`).
fn is_within(pointer: &str, entity: &str) -> bool {
    pointer == entity
        || pointer
            .strip_prefix(entity)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// The error for a candidate the importer refused, with each finding attributed to the ops that
/// changed the node or clip it points into.
fn rejected(error: ImportError, touched: &[(usize, String)]) -> EditError {
    let issues: Vec<LocalizedIssue> = error
        .issues()
        .iter()
        .map(|issue| LocalizedIssue {
            ops: touched
                .iter()
                .filter(|(_, entity)| is_within(&issue.path, entity))
                .map(|(index, _)| *index)
                .collect(),
            issue: issue.clone(),
        })
        .collect();
    let index = issues.iter().find_map(|found| found.ops.first().copied());
    let mut failure = EditError::new(
        CANDIDATE_REJECTED,
        format!("the edited scene is not a valid bundle: {error}"),
    )
    .rejected_by(issues, error);
    failure.index = index;
    failure
}

/// Applies the batch with the importer's default `Limits`. See `apply_with`.
pub fn apply(scene: &DirectedScene, ops: &[EditOp]) -> Result<EditOutcome, EditError> {
    apply_with(scene, ops, &Limits::default())
}

/// Applies the ops in order to a copy of the scene's document and revalidates the result.
///
/// Per op: the scene, node or clip must exist (`UNKNOWN_*`), the property must apply to that
/// node (`NOT_APPLICABLE`), and the value must have the property's kind (`INVALID_TYPE`), be
/// finite and in range (`INVALID_VALUE`); a `reset` needs a catalog default (`NO_DEFAULT`). The
/// first op that fails ends the batch with its position and path. Later ops see the values
/// earlier ones wrote.
///
/// After the last op the whole candidate goes through `DirectedScene::import_value_with`, the
/// importer that already owns the integrity rules (unit-length quaternions, finite world
/// matrices, camera rules); its findings come back as `CANDIDATE_REJECTED` with each issue's JSON
/// Pointer and the ops that touched it. Structural joins and temporal camera gates run on the
/// final candidate as part of this same atomic validation. `limits` should be the ones the scene
/// was imported with.
///
/// On any error nothing exists besides the error: `scene` is borrowed immutably.
pub fn apply_with(
    scene: &DirectedScene,
    ops: &[EditOp],
    limits: &Limits,
) -> Result<EditOutcome, EditError> {
    let mut work = scene.document().clone();
    let mut locator = Locator::new(scene);
    let mut changes = Vec::with_capacity(ops.len());
    let mut touched: Vec<(usize, String)> = Vec::new();
    let mut conversions: Vec<MovementConversion> = Vec::new();
    // The datum each op wrote (a component is read as its parent vector): the net effect is judged
    // on these, in the input and in the result, not on the sequence of paths the ops spelled.
    let mut data: Vec<Datum> = Vec::new();

    for (index, op) in ops.iter().enumerate() {
        let property = op.property();
        let path = property.path();
        let fail = |error: EditError| error.at_op(index).with_path(&path);

        let located = locator.locate(property).map_err(fail)?;
        let before = read_value(&work, &located).map_err(fail)?;
        let after = match op {
            EditOp::Set { value, .. } => {
                located.spec.validate(value).map_err(fail)?;
                *value
            }
            EditOp::Reset { .. } => located.spec.default.ok_or_else(|| {
                fail(EditError::new(
                    NO_DEFAULT,
                    format!("`{}` has no default to reset to", located.spec.name),
                ))
            })?,
        };
        if before != after {
            // The origin's write of a camera start: the first movement becomes keys, from the
            // start the clip has right now, before the start moves. A write that changes nothing
            // converts nothing (the document is left alone).
            if let (true, Some(slot)) = (located.spec.converts_first_movement, located.clip_slot) {
                let base = &scene.bundle().camera_clips[slot];
                let converted = convert_first_movement(&mut work, &located.entity, base, index)
                    .map_err(fail)?;
                conversions.extend(converted);
            }
            write_value(&mut work, &located, &after).map_err(fail)?;
            note_datum(&mut data, &located);
            touched.push((index, located.entity.clone()));
        }
        changes.push(PropertyChange {
            index,
            property: property.clone(),
            before,
            after,
        });
    }

    let modified = net_modified(scene.document(), &work, &data)?;
    let candidate = DirectedScene::import_value_with(work, limits)
        .map_err(|error| rejected(error, &touched))?;
    let mut issues = candidate.edit_join_issues();
    issues.extend(candidate.temporal_issues());
    if !issues.is_empty() {
        return Err(rejected(ImportError::Invalid { issues }, &touched));
    }
    let candidate = if modified {
        candidate
    } else {
        // WHY the input back: the importer accepted `work`, but a net no-op must not carry what
        // the ops did on the way (keys from a conversion, a restored number spelled `1.0`).
        scene.clone()
    };
    Ok(EditOutcome {
        candidate,
        summary: EditSummary::new(changes, conversions, modified),
    })
}

/// What a write changed in the document: the parent property when the op wrote a component.
struct Datum {
    field: String,
    spec: &'static PropertySpec,
}

fn note_datum(data: &mut Vec<Datum>, located: &Located) {
    let spec = match located.spec.component {
        Some(component) => component.parent.spec(),
        None => located.spec,
    };
    let field = format!("{}/{}", located.entity, spec.document_field);
    if !data.iter().any(|datum| datum.field == field) {
        data.push(Datum { field, spec });
    }
}

/// Whether any datum the batch wrote ends with a value other than the one the input held.
fn net_modified(input: &Value, work: &Value, data: &[Datum]) -> Result<bool, EditError> {
    for datum in data {
        let located = Located {
            entity: String::new(),
            field: datum.field.clone(),
            spec: datum.spec,
            clip_slot: None,
        };
        if read_value(input, &located)? != read_value(work, &located)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Validates the batch exactly as `apply` would and reports what it would change, without
/// returning the candidate. Succeeds if and only if `apply` does, with the same summary.
pub fn dry_run(scene: &DirectedScene, ops: &[EditOp]) -> Result<EditSummary, EditError> {
    dry_run_with(scene, ops, &Limits::default())
}

pub fn dry_run_with(
    scene: &DirectedScene,
    ops: &[EditOp],
    limits: &Limits,
) -> Result<EditSummary, EditError> {
    apply_with(scene, ops, limits).map(|outcome| outcome.summary)
}

/// A property of the scene with the value it holds now.
#[derive(Debug, Clone, PartialEq)]
pub struct PropertyInfo {
    pub property: PropertyRef,
    pub value: PropertyValue,
}

impl PropertyInfo {
    pub fn spec(&self) -> &'static PropertySpec {
        self.property.spec()
    }

    pub fn entity(&self) -> Entity {
        match &self.property {
            PropertyRef::Node {
                scene_id, node_id, ..
            } => Entity::node(scene_id, node_id),
            PropertyRef::Camera { clip_id, .. } => Entity::new(EntityKind::CameraClip, clip_id),
        }
    }
}

/// The current value of one property, with the same ID and applicability errors a `set` gives.
pub fn read_property(
    scene: &DirectedScene,
    property: &PropertyRef,
) -> Result<PropertyValue, EditError> {
    let located = Locator::new(scene).locate(property)?;
    read_value(scene.document(), &located)
}

/// Every property that applies to the scene's nodes and camera clips, with its current value:
/// nodes in document order (scene by scene), then clips. What an inspector lists, with no
/// selection involved. Components (`position/x`) are not listed: they restate their parent; see
/// `properties_with_components`.
pub fn properties_of(scene: &DirectedScene) -> Vec<PropertyInfo> {
    list_properties(scene, false)
}

/// `properties_of` plus the component properties (`position/x`, `start/target/z`, ...), after the
/// properties the same node or clip lists on its own.
pub fn properties_with_components(scene: &DirectedScene) -> Vec<PropertyInfo> {
    list_properties(scene, true)
}

fn list_properties(scene: &DirectedScene, components: bool) -> Vec<PropertyInfo> {
    let bundle = scene.bundle();
    let listed = |spec: &PropertySpec| components || spec.component.is_none();
    let mut properties = Vec::new();
    for graph in &bundle.scenes {
        for node in graph.nodes() {
            for property in NodeProperty::ALL {
                let spec = property.spec();
                if listed(spec) && spec.applies_to.accepts_node(node.kind) {
                    properties.push(PropertyRef::node(graph.id(), &node.id, property));
                }
            }
        }
    }
    for clip in &bundle.camera_clips {
        for property in CameraProperty::ALL {
            if listed(property.spec()) {
                properties.push(PropertyRef::camera(&clip.id, property));
            }
        }
    }
    let mut locator = Locator::new(scene);
    properties
        .into_iter()
        .filter_map(|property| {
            let located = locator.locate(&property).ok()?;
            let value = read_value(scene.document(), &located).ok()?;
            Some(PropertyInfo { property, value })
        })
        .collect()
}
