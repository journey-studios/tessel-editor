//! Editing `fx.transitions` (F064, P4b S3a): the five transition ops of `scene.fx.edit`, their JSON
//! shapes and what each does to the document.
//!
//! They are ops of the SAME batch as the looks (`FxOp::Transition`): one copy of the document, one
//! reimport of the candidate by the importer (which keeps the admission policy of the scene and
//! owns every rule: window, neighbours, overlap, handles, quotas), one undo entry. This module
//! decides only what an op writes, never whether the result is legal:
//!
//! * `set_transition` is an upsert **by cut** (the exact pair `fromShotId`, `toShotId`). On an
//!   existing transition it keeps the `id` and every field the op does not name (an explicit `id`
//!   that is not the existing one is refused, `TRANSITION_ID_MISMATCH`, not ignored and not a
//!   rename), merges `params` key by key and leaves members it does not know alone. On a new cut it
//!   writes the whole transition with the contract defaults and appends it.
//! * `resize_transition`, `set_transition_param`, `set_transition_hidden` and `remove_transition`
//!   name a transition by `id`; an `id` that is not there is `UNKNOWN_TRANSITION`.
//! * A no-op is judged on typed values (`1` and `1.0` are one number) and leaves the authored
//!   spelling alone. `hidden: false` is written as no `hidden` member, which is the only form the
//!   importer accepts.
//!
//! Op-level problems carry structured `args`, and their English text is the projection of the
//! same table the importer's issues use (`fx::project_problem`), so a transition problem reads the
//! same wherever it is found.

use serde_json::{Map, Value, json};

use crate::directed_scene::Shot;
use crate::directed_scene::fx::{
    self, Alignment, DUPLICATE_ID, FX_EASINGS, FxEasing, FxValue, Handles, Locale, RawProblem,
    TRANSITION_ID_MISMATCH, TRANSITION_TOO_LONG, UNKNOWN_EFFECT, UNKNOWN_KEYFRAME_EASING,
    UNKNOWN_PARAM, UNKNOWN_SHOT, UNKNOWN_TRANSITION, WRONG_FAMILY, default_transition_duration,
    param_fault_args, parse_param, project_problem,
};

use super::error::{CANDIDATE_REJECTED, EditError, INVALID_OP};
use super::fx::{EditSpan, FxChange, bad_id, every_id, fresh_id, mismatch, text_of, tracks_mut};
use super::fx_op::FxOp;

/// The contract's default easing for a new transition.
const DEFAULT_EASING: FxEasing = FxEasing::EaseInOutCubic;

/// The prefix of a generated transition id (`tr-1`, `tr-2`, ...).
const ID_PREFIX: &str = "tr";

pub(super) const SHAPES: &str = r#"{ "op": "set_transition", "fromShotId", "toShotId", "effect": "crossDissolve", "id"?, "durationFrames"?, "alignment"?: "center|start|end", "handles"?: "use|hold|refuse", "easing"?, "params"?, "hidden"? }, { "op": "resize_transition", "id", "durationFrames", "alignment"? }, { "op": "set_transition_param", "id", "param", "value" }, { "op": "set_transition_hidden", "id", "hidden" } or { "op": "remove_transition", "id" }"#;

/// A transition op, parsed and typed (the names are the document's, in camelCase).
#[derive(Debug, Clone, PartialEq)]
pub enum TransitionOp {
    /// Upsert by cut.
    Set(SetTransition),
    /// A new window for an existing transition.
    Resize {
        id: String,
        duration_frames: i64,
        alignment: Option<Alignment>,
    },
    /// One parameter of the effect.
    SetParam {
        id: String,
        param: String,
        value: Value,
    },
    /// `true` writes `hidden: true`; `false` leaves the member out.
    SetHidden { id: String, hidden: bool },
    /// Removes the transition; the others keep their order.
    Remove { id: String },
}

/// `set_transition`: what to write on the cut `from_shot_id` to `to_shot_id`. Every optional field
/// left out keeps what the cut has, or takes the contract's default on a new transition.
#[derive(Debug, Clone, PartialEq)]
pub struct SetTransition {
    pub id: Option<String>,
    pub from_shot_id: String,
    pub to_shot_id: String,
    pub effect: String,
    pub duration_frames: Option<i64>,
    pub alignment: Option<Alignment>,
    pub handles: Option<Handles>,
    /// The name; judged when the op runs, like every other value.
    pub easing: Option<String>,
    pub params: Vec<(String, Value)>,
    pub hidden: Option<bool>,
}

impl TransitionOp {
    /// The transition an op names by `id` (a `set_transition` names a cut, not an id, unless it
    /// gives one).
    pub fn transition_id(&self) -> Option<&str> {
        match self {
            Self::Set(set) => set.id.as_deref(),
            Self::Resize { id, .. }
            | Self::SetParam { id, .. }
            | Self::SetHidden { id, .. }
            | Self::Remove { id } => Some(id),
        }
    }

    /// The JSON of the five ops; `None` when `op` is not one of them (the caller reports it).
    pub(super) fn from_json(op: &str, map: &Map<String, Value>) -> Option<Result<Self, EditError>> {
        let result = match op {
            "set_transition"
            | "resize_transition"
            | "set_transition_param"
            | "set_transition_hidden"
            | "remove_transition" => parse(op, map),
            _ => return None,
        };
        Some(result)
    }
}

fn parse(op: &str, map: &Map<String, Value>) -> Result<TransitionOp, EditError> {
    let invalid =
        |message: String| EditError::new(INVALID_OP, format!("{message}; expected {SHAPES}"));
    let text = |key: &str| -> Result<String, EditError> {
        map.get(key)
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| invalid(format!("`{key}` must be a string")))
    };
    let optional_text = |key: &str| -> Result<Option<String>, EditError> {
        match map.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(text)) => Ok(Some(text.clone())),
            Some(_) => Err(invalid(format!("`{key}` must be a string"))),
        }
    };
    // Strict: `12.0`, `12.5`, text and booleans are not whole numbers.
    let integer = |key: &str| -> Result<i64, EditError> {
        map.get(key)
            .and_then(Value::as_i64)
            .ok_or_else(|| invalid(format!("`{key}` must be a whole number")))
    };
    let optional_integer = |key: &str| -> Result<Option<i64>, EditError> {
        match map.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(_) => integer(key).map(Some),
        }
    };
    let boolean = |key: &str| -> Result<bool, EditError> {
        map.get(key)
            .and_then(Value::as_bool)
            .ok_or_else(|| invalid(format!("`{key}` must be true or false")))
    };
    let only = |keys: &[&str]| -> Result<(), EditError> {
        match map.keys().find(|key| !keys.contains(&key.as_str())) {
            Some(extra) => Err(invalid(format!("`{extra}` is not a field of this op"))),
            None => Ok(()),
        }
    };
    let alignment = |key: &str| -> Result<Option<Alignment>, EditError> {
        optional_text(key)?
            .map(|name| {
                Alignment::parse(&name).ok_or_else(|| {
                    invalid(format!(
                        "`{name}` is not an alignment ({})",
                        Alignment::NAMES.join(", ")
                    ))
                })
            })
            .transpose()
    };
    match op {
        "set_transition" => {
            only(&[
                "op",
                "id",
                "fromShotId",
                "toShotId",
                "effect",
                "durationFrames",
                "alignment",
                "handles",
                "easing",
                "params",
                "hidden",
            ])?;
            let handles = optional_text("handles")?
                .map(|name| {
                    Handles::parse(&name).ok_or_else(|| {
                        invalid(format!(
                            "`{name}` is not a handles policy ({})",
                            Handles::NAMES.join(", ")
                        ))
                    })
                })
                .transpose()?;
            let params = match map.get("params") {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Object(params)) => params
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect(),
                Some(_) => return Err(invalid("`params` must be an object".to_string())),
            };
            let hidden = match map.get("hidden") {
                None | Some(Value::Null) => None,
                Some(_) => Some(boolean("hidden")?),
            };
            Ok(TransitionOp::Set(SetTransition {
                id: optional_text("id")?,
                from_shot_id: text("fromShotId")?,
                to_shot_id: text("toShotId")?,
                effect: text("effect")?,
                duration_frames: optional_integer("durationFrames")?,
                alignment: alignment("alignment")?,
                handles,
                easing: optional_text("easing")?,
                params,
                hidden,
            }))
        }
        "resize_transition" => {
            only(&["op", "id", "durationFrames", "alignment"])?;
            Ok(TransitionOp::Resize {
                id: text("id")?,
                duration_frames: integer("durationFrames")?,
                alignment: alignment("alignment")?,
            })
        }
        "set_transition_param" => {
            only(&["op", "id", "param", "value"])?;
            Ok(TransitionOp::SetParam {
                id: text("id")?,
                param: text("param")?,
                value: map
                    .get("value")
                    .cloned()
                    .ok_or_else(|| invalid("`value` is required".to_string()))?,
            })
        }
        "set_transition_hidden" => {
            only(&["op", "id", "hidden"])?;
            Ok(TransitionOp::SetHidden {
                id: text("id")?,
                hidden: boolean("hidden")?,
            })
        }
        _ => {
            only(&["op", "id"])?;
            Ok(TransitionOp::Remove { id: text("id")? })
        }
    }
}

/// An op-level problem: the code, structured `args` and the English text of the shared table. The
/// hint, `args` and the transition travel in the error's `details`.
fn problem(
    code: &'static str,
    transition: Option<&str>,
    path: Option<&str>,
    args: Value,
) -> EditError {
    let mut details = json!({ "args": args });
    if let Some(id) = transition {
        details["transitionId"] = json!(id);
    }
    let found = project_problem(
        &RawProblem {
            code,
            path,
            message: "",
            details: Some(&details),
        },
        Locale::En,
    );
    if let Some(hint) = &found.hint {
        details["hint"] = json!(hint);
    }
    let mut error = EditError::new(code, found.message).with_details(details);
    if let Some(path) = path {
        error = error.with_path(path);
    }
    if let Some(id) = transition {
        error = error.with_id(id);
    }
    error
}

fn transitions(work: &Value) -> &[Value] {
    work.pointer("/fx/transitions")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn transitions_mut(work: &mut Value) -> Result<&mut Vec<Value>, EditError> {
    // The layer exists from here on (an edit that adds the first transition to a document with no
    // `fx`); a batch that ends with it bare takes it out again.
    tracks_mut(work)?;
    work.pointer_mut("/fx/transitions")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| mismatch("`/fx/transitions` is not an array"))
}

fn position_of(work: &Value, id: &str) -> Option<usize> {
    transitions(work)
        .iter()
        .position(|transition| text_of(transition, "id") == id)
}

/// The JSON Pointer of a transition by id, for the problems the importer finds on it.
pub(super) fn pointer_of(work: &Value, id: &str) -> Option<String> {
    position_of(work, id).map(|slot| format!("/fx/transitions/{slot}"))
}

fn locate(work: &Value, id: &str) -> Result<usize, EditError> {
    position_of(work, id).ok_or_else(|| {
        let known: Vec<&str> = transitions(work)
            .iter()
            .map(|transition| text_of(transition, "id"))
            .collect();
        let kind = if known.is_empty() { "none" } else { "some" };
        problem(
            UNKNOWN_TRANSITION,
            Some(id),
            Some("id"),
            json!({ "kind": kind, "known": known }),
        )
    })
}

fn object_at(work: &mut Value, slot: usize) -> Result<&mut Map<String, Value>, EditError> {
    transitions_mut(work)?
        .get_mut(slot)
        .and_then(Value::as_object_mut)
        .ok_or_else(|| mismatch("a transition is not an object"))
}

fn lengths(shots: &[Shot], from: &str, to: &str) -> Option<(i64, i64)> {
    let length = |id: &str| {
        shots
            .iter()
            .find(|shot| shot.id == id)
            .map(|shot| i64::try_from(shot.duration_frames()).unwrap_or(i64::MAX))
    };
    Some((length(from)?, length(to)?))
}

/// The effect of a transition by name: a look is `WRONG_FAMILY` and no effect at all is
/// `UNKNOWN_EFFECT`, as the importer says it.
fn effect_of(name: &str, transition: &str) -> Result<&'static fx::EffectSpec, EditError> {
    if let Some(spec) = fx::transition_effect(name) {
        return Ok(spec);
    }
    Err(match fx::effect(name) {
        Some(look) => problem(
            WRONG_FAMILY,
            Some(transition),
            Some("effect"),
            json!({ "effect": name, "family": look.family }),
        ),
        None => problem(
            UNKNOWN_EFFECT,
            Some(transition),
            Some("effect"),
            json!({ "effect": name }),
        ),
    })
}

/// A parameter of the effect, typed: unknown, of the wrong type or out of range are refused with
/// what the effect accepts.
fn param_of(
    spec: &'static fx::EffectSpec,
    transition: &str,
    name: &str,
    value: &Value,
) -> Result<FxValue, EditError> {
    let Some((_, param)) = spec.param(name) else {
        let known = spec
            .params
            .iter()
            .map(|param| param.id)
            .collect::<Vec<_>>()
            .join(", ");
        return Err(problem(
            UNKNOWN_PARAM,
            Some(transition),
            Some(name),
            json!({ "effect": spec.id, "param": name, "known": known }),
        ));
    };
    parse_param(param, value).map_err(|(code, _)| {
        problem(
            code,
            Some(transition),
            Some(name),
            param_fault_args(param, code, name, value),
        )
    })
}

/// What the document spells for a parameter, typed; `None` when it leaves it out (it then reads as
/// the default, but writing the default down is still a change of the document).
fn written_param(
    spec: &'static fx::EffectSpec,
    object: &Map<String, Value>,
    name: &str,
) -> Option<FxValue> {
    let (_, param) = spec.param(name)?;
    object
        .get("params")
        .and_then(|params| params.get(name))
        .and_then(|raw| parse_param(param, raw).ok())
}

/// Restore authored parameter spelling after a batch returns to the same typed value.
/// Presence is significant: explicitly writing an omitted default remains a real edit.
pub(super) fn restore_authored_params(original: &Value, work: &mut Value) {
    let authored: std::collections::HashMap<_, _> = transitions(original)
        .iter()
        .filter_map(|item| Some((item.get("id")?.as_str()?, item)))
        .collect();
    let Some(edited) = work
        .pointer_mut("/fx/transitions")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for item in edited {
        let Some(prior) = item
            .get("id")
            .and_then(Value::as_str)
            .and_then(|id| authored.get(id))
        else {
            continue;
        };
        if ["effect", "fromShotId", "toShotId"]
            .iter()
            .any(|key| item.get(*key) != prior.get(*key))
        {
            continue;
        }
        let Some(spec) = prior
            .get("effect")
            .and_then(Value::as_str)
            .and_then(fx::transition_effect)
        else {
            continue;
        };
        let Some(params) = item.get_mut("params").and_then(Value::as_object_mut) else {
            continue;
        };
        for param in spec.params {
            let Some(before) = prior.get("params").and_then(|p| p.get(param.id)) else {
                continue;
            };
            let Some(after) = params.get(param.id) else {
                continue;
            };
            if before != after
                && parse_param(param, before)
                    .ok()
                    .is_some_and(|value| parse_param(param, after) == Ok(value))
            {
                params.insert(param.id.to_string(), before.clone());
            }
        }
    }
}

fn easing_of(name: &str, transition: &str) -> Result<FxEasing, EditError> {
    FxEasing::parse(name).ok_or_else(|| {
        problem(
            UNKNOWN_KEYFRAME_EASING,
            Some(transition),
            Some("easing"),
            json!({ "easing": name, "known": FX_EASINGS.join(", ") }),
        )
    })
}

fn change(
    index: usize,
    action: &'static str,
    id: &str,
    target: Option<&str>,
    values: (Option<Value>, Option<Value>),
    noop: bool,
) -> FxChange {
    FxChange {
        index,
        warnings: Vec::new(),
        action,
        track: String::new(),
        clip: id.to_string(),
        transition: Some(id.to_string()),
        target: target.map(str::to_string),
        before: values.0,
        after: values.1,
        noop,
    }
}

impl TransitionOp {
    pub(super) fn apply(
        &self,
        work: &mut Value,
        index: usize,
        span: EditSpan<'_>,
    ) -> Result<FxChange, EditError> {
        match self {
            Self::Set(set) => set_transition(work, set, index, span),
            Self::Resize {
                id,
                duration_frames,
                alignment,
            } => resize(work, id, *duration_frames, *alignment, index),
            Self::SetParam { id, param, value } => set_param(work, id, param, value, index),
            Self::SetHidden { id, hidden } => set_hidden(work, id, *hidden, index),
            Self::Remove { id } => remove(work, id, index),
        }
    }
}

fn unknown_shot(shots: &[Shot], wanted: &str, transition: &str, key: &str) -> EditError {
    let mut ordered: Vec<&Shot> = shots.iter().collect();
    ordered.sort_by_key(|shot| shot.start_frame);
    problem(
        UNKNOWN_SHOT,
        Some(transition),
        Some(key),
        json!({
            "shot": wanted,
            "shots": ordered.iter().map(|shot| shot.id.as_str()).collect::<Vec<_>>(),
        }),
    )
}

fn set_transition(
    work: &mut Value,
    set: &SetTransition,
    index: usize,
    span: EditSpan<'_>,
) -> Result<FxChange, EditError> {
    let (from, to) = (set.from_shot_id.as_str(), set.to_shot_id.as_str());
    let Some((from_length, to_length)) = lengths(span.shots, from, to) else {
        let (wanted, key) = if span.shots.iter().any(|shot| shot.id == from) {
            (to, "toShotId")
        } else {
            (from, "fromShotId")
        };
        // The sentence names the transition: the one the op asked for, or the id it would have
        // been given (nothing exists on a cut that has a shot that is not there).
        let id = set
            .id
            .clone()
            .unwrap_or_else(|| fresh_id(&every_id(work), ID_PREFIX));
        return Err(unknown_shot(span.shots, wanted, &id, key));
    };
    let slot = transitions(work).iter().position(|transition| {
        text_of(transition, "fromShotId") == from && text_of(transition, "toShotId") == to
    });
    let existing = slot.map(|slot| text_of(&transitions(work)[slot], "id").to_string());
    let ids = every_id(work);
    let id = match (&set.id, &existing) {
        (Some(explicit), Some(found)) if explicit != found => {
            return Err(problem(
                TRANSITION_ID_MISMATCH,
                Some(explicit),
                Some("id"),
                json!({ "existing": found, "from": from, "to": to }),
            ));
        }
        (_, Some(found)) => found.clone(),
        (Some(explicit), None) => {
            if let Some(error) = bad_id(explicit) {
                return Err(error.with_path("id"));
            }
            if ids.contains(explicit) {
                return Err(EditError::new(
                    DUPLICATE_ID,
                    format!("an fx track, clip or transition `{explicit}` already exists"),
                )
                .with_id(explicit.as_str())
                .with_path("id"));
            }
            explicit.clone()
        }
        (None, None) => fresh_id(&ids, ID_PREFIX),
    };
    let spec = effect_of(&set.effect, &id)?;
    let easing = set
        .easing
        .as_deref()
        .map(|name| easing_of(name, &id))
        .transpose()?;
    let mut typed: Vec<(&str, FxValue)> = Vec::with_capacity(set.params.len());
    let mut named: Vec<&(String, Value)> = set.params.iter().collect();
    named.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, value) in named {
        typed.push((name.as_str(), param_of(spec, &id, name, value)?));
    }

    let Some(slot) = slot else {
        let alignment = set.alignment.unwrap_or(Alignment::Center);
        let duration = match set.duration_frames {
            Some(duration) => duration,
            None => default_transition_duration(
                spec.typical_duration_frames,
                alignment,
                from_length,
                to_length,
            )
            .ok_or_else(|| {
                problem(
                    TRANSITION_TOO_LONG,
                    Some(&id),
                    None,
                    json!({
                        "kind": "no_room", "from": from, "to": to,
                        "fromLength": from_length, "toLength": to_length,
                        "alignment": alignment.name(),
                    }),
                )
            })?,
        };
        let mut params = Map::new();
        for param in spec.params {
            let value = typed
                .iter()
                .find(|(name, _)| *name == param.id)
                .map_or_else(|| param.kind.default_value(), |(_, value)| *value);
            params.insert(param.id.to_string(), value.to_json());
        }
        let mut written = Map::new();
        written.insert("id".to_string(), json!(id));
        written.insert("effect".to_string(), json!(spec.id));
        written.insert("fromShotId".to_string(), json!(from));
        written.insert("toShotId".to_string(), json!(to));
        written.insert("durationFrames".to_string(), json!(duration));
        written.insert("alignment".to_string(), json!(alignment.name()));
        written.insert(
            "handles".to_string(),
            json!(set.handles.unwrap_or(Handles::Hold).name()),
        );
        written.insert(
            "easing".to_string(),
            json!(easing.unwrap_or(DEFAULT_EASING).name()),
        );
        written.insert("params".to_string(), Value::Object(params));
        if set.hidden == Some(true) {
            written.insert("hidden".to_string(), Value::Bool(true));
        }
        let written = Value::Object(written);
        transitions_mut(work)?.push(written.clone());
        return Ok(change(
            index,
            "set_transition",
            &id,
            None,
            (None, Some(written)),
            false,
        ));
    };

    let object = object_at(work, slot)?;
    let before = Value::Object(object.clone());
    let mut changed = put(object, "effect", json!(spec.id));
    if let Some(duration) = set.duration_frames {
        changed |= put(object, "durationFrames", json!(duration));
    }
    if let Some(alignment) = set.alignment {
        changed |= put(object, "alignment", json!(alignment.name()));
    }
    if let Some(handles) = set.handles {
        changed |= put(object, "handles", json!(handles.name()));
    }
    if let Some(easing) = easing {
        changed |= put(object, "easing", json!(easing.name()));
    }
    for (name, value) in &typed {
        // The very value already written is left in its authored spelling.
        if written_param(spec, object, name) == Some(*value) {
            continue;
        }
        if object
            .get("params")
            .is_some_and(|params| !params.is_object())
        {
            return Err(mismatch("`params` of a transition is not an object"));
        }
        let params = object
            .entry("params")
            .or_insert_with(|| Value::Object(Map::new()));
        if let Some(params) = params.as_object_mut() {
            params.insert((*name).to_string(), value.to_json());
            changed = true;
        }
    }
    match set.hidden {
        Some(true) => changed |= put(object, "hidden", Value::Bool(true)),
        Some(false) => changed |= object.remove("hidden").is_some(),
        None => {}
    }
    let after = Value::Object(object.clone());
    Ok(change(
        index,
        "set_transition",
        &id,
        None,
        (Some(before), Some(after)),
        !changed,
    ))
}

/// Writes `value` at `key` unless it is already there; whether the document changed.
fn put(object: &mut Map<String, Value>, key: &str, value: Value) -> bool {
    if object.get(key) == Some(&value) {
        return false;
    }
    object.insert(key.to_string(), value);
    true
}

fn resize(
    work: &mut Value,
    id: &str,
    duration: i64,
    alignment: Option<Alignment>,
    index: usize,
) -> Result<FxChange, EditError> {
    let slot = locate(work, id)?;
    let object = object_at(work, slot)?;
    let window = |object: &Map<String, Value>| {
        json!({
            "durationFrames": object.get("durationFrames"),
            "alignment": object.get("alignment"),
        })
    };
    let before = window(object);
    let mut changed = false;
    if object.get("durationFrames").and_then(Value::as_i64) != Some(duration) {
        object.insert("durationFrames".to_string(), json!(duration));
        changed = true;
    }
    if let Some(alignment) = alignment
        && object.get("alignment").and_then(Value::as_str) != Some(alignment.name())
    {
        object.insert("alignment".to_string(), json!(alignment.name()));
        changed = true;
    }
    let after = window(object);
    Ok(change(
        index,
        "resize_transition",
        id,
        Some("window"),
        (Some(before), Some(after)),
        !changed,
    ))
}

fn set_param(
    work: &mut Value,
    id: &str,
    param: &str,
    value: &Value,
    index: usize,
) -> Result<FxChange, EditError> {
    let slot = locate(work, id)?;
    let object = object_at(work, slot)?;
    let effect = object
        .get("effect")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let spec = effect_of(&effect, id)?;
    let after = param_of(spec, id, param, value)?;
    let catalog_default = spec
        .param(param)
        .map(|(_, param)| param.kind.default_value());
    let written = written_param(spec, object, param);
    let before = written.or(catalog_default);
    let noop = written == Some(after);
    if !noop {
        if object
            .get("params")
            .is_some_and(|params| !params.is_object())
        {
            return Err(mismatch("`params` of a transition is not an object"));
        }
        let params = object
            .entry("params")
            .or_insert_with(|| Value::Object(Map::new()));
        if let Some(params) = params.as_object_mut() {
            params.insert(param.to_string(), after.to_json());
        }
    }
    Ok(change(
        index,
        "set_transition_param",
        id,
        Some(param),
        (before.map(|value| value.to_json()), Some(after.to_json())),
        noop,
    ))
}

fn set_hidden(
    work: &mut Value,
    id: &str,
    hidden: bool,
    index: usize,
) -> Result<FxChange, EditError> {
    let slot = locate(work, id)?;
    let object = object_at(work, slot)?;
    let before = object.get("hidden") == Some(&Value::Bool(true));
    if hidden != before {
        if hidden {
            object.insert("hidden".to_string(), Value::Bool(true));
        } else {
            object.remove("hidden");
        }
    }
    Ok(change(
        index,
        "set_transition_hidden",
        id,
        Some("hidden"),
        (Some(json!(before)), Some(json!(hidden))),
        hidden == before,
    ))
}

fn remove(work: &mut Value, id: &str, index: usize) -> Result<FxChange, EditError> {
    let slot = locate(work, id)?;
    let removed = transitions_mut(work)?.remove(slot);
    Ok(change(
        index,
        "remove_transition",
        id,
        None,
        (Some(removed), None),
        false,
    ))
}

/// Whether an op is one of the five.
pub(super) fn is_transition(op: &FxOp) -> bool {
    matches!(op, FxOp::Transition(_))
}

/// The problems of a refused batch in the shape `{ code, message, hint, transitionId?, shotId?,
/// details }`, in English (the executor has no locale): the importer's findings for a rejected
/// candidate, or the one problem of the failing op.
fn problems_of(error: &EditError) -> Vec<Value> {
    if error.code == CANDIDATE_REJECTED {
        return error
            .issues()
            .iter()
            .map(|found| {
                let mut problem =
                    project_problem(&RawProblem::from_issue(&found.issue), Locale::En).to_json();
                problem["details"]["ops"] = json!(found.ops);
                problem
            })
            .collect();
    }
    let raw = RawProblem {
        code: error.code,
        path: error.path.as_deref(),
        message: &error.message,
        details: error.details(),
    };
    vec![project_problem(&raw, Locale::En).to_json()]
}

/// Adds `details.errors[]` to the refusal of a batch that carries a transition op: the failing op
/// was one of them, or the importer refused the candidate. A refusal of looks only, and every
/// success (no-ops included), stay exactly as they were.
pub(super) fn attach_problems(error: EditError, ops: &[FxOp]) -> EditError {
    let rejected = error.code == CANDIDATE_REJECTED && ops.iter().any(is_transition);
    let failed = error.code != CANDIDATE_REJECTED
        && error
            .index
            .and_then(|index| ops.get(index))
            .is_some_and(is_transition);
    if !(rejected || failed) {
        return error;
    }
    let mut details = match error.details() {
        Some(Value::Object(facts)) => facts.clone(),
        _ => Map::new(),
    };
    details.insert("errors".to_string(), Value::Array(problems_of(&error)));
    error.with_details(Value::Object(details))
}
