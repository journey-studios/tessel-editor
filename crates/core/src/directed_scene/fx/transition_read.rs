//! Reading `fx.transitions` (F064, P4b S1a), only under [`TransitionAdmission::Accept`].
//!
//! Two kinds of problem, in two places, so the refusals come out in the order the contract fixes
//! (plan §5.2) and the way the pilot's `explainFx` reports them, all together:
//!
//! * **structure and types** (step 1) go to the importer's `Sink` like any other shape error: a
//!   missing field, a wrong type, a float where an integer goes, `hidden: false`, an ID that breaks
//!   the rule, `keys` at all. The import stops there, as for every other section.
//! * **effect, family, params, easing and the enums** (steps 2 and 3) are kept in the
//!   [`TransitionDraft`] as [`Issue`]s and reported by the explain stage, ahead of the shot,
//!   adjacency, window, handle and overlap rules of the same transition (steps 4 to 11), which need
//!   the whole bundle and so run after it is built.
//!
//! A transition is admitted only when no step finds anything: import is all-or-nothing.

use serde_json::{Map, Value, json};

use crate::directed_scene::issue::{INVALID_TYPE, INVALID_VALUE, Issue};
use crate::directed_scene::read::{Arr, Bound, Obj, Sink, child, kind_of};

use super::catalog::{EffectSpec, ParamKind};
use super::read::parse_param;
use super::{
    Alignment, FX_EASINGS, FX_LIMIT, FX_TRANSITION_KEYS_NOT_YET, FxEasing, FxValue, Handles,
    PARAM_OUT_OF_RANGE, PARAM_TYPE, UNKNOWN_EFFECT, UNKNOWN_KEYFRAME_EASING, UNKNOWN_PARAM,
    WRONG_FAMILY, effect, id_problem, transition_effect,
};

/// What the explain stage needs of one transition that passed the structure step.
pub(in crate::directed_scene) struct TransitionDraft {
    /// Position in `fx.transitions`, for the JSON Pointer of later issues.
    pub(super) index: usize,
    pub(super) id: String,
    pub(super) from_shot_id: String,
    pub(super) to_shot_id: String,
    pub(super) duration_frames: i64,
    /// `None` when `alignment` is not one of the three names (reported in `early`): the window
    /// and everything after it cannot be judged.
    pub(super) alignment: Option<Alignment>,
    pub(super) handles: Option<Handles>,
    /// The issues of steps 2 and 3 (effect, family, params, easing, enums), in that order.
    pub(super) early: Vec<Issue>,
    /// The typed transition, present exactly when `early` is empty.
    pub(super) resolved: Option<Resolved>,
}

pub(super) struct Resolved {
    pub(super) effect: &'static EffectSpec,
    pub(super) easing: FxEasing,
    pub(super) params: Vec<FxValue>,
    pub(super) seed: Option<u32>,
    pub(super) hidden: bool,
}

/// A number as the origin spells it in a message: an integral float without its `.0`.
fn spelled(number: &serde_json::Number) -> String {
    match number.as_f64() {
        Some(float) if float.is_finite() && float.fract() == 0.0 && float.abs() < 1e21 => {
            format!("{float:.0}")
        }
        _ => number.to_string(),
    }
}

/// `0` and `1` as the origin prints them, not `0.0` and `1.0`.
fn plain(float: f64) -> String {
    if float.fract() == 0.0 && float.abs() < 1e21 {
        format!("{float:.0}")
    } else {
        float.to_string()
    }
}

/// What a parameter accepts, as the hint of its issues says it.
fn describe(kind: &ParamKind) -> String {
    match *kind {
        ParamKind::Number {
            min,
            max,
            default,
            unit,
            ..
        } => format!(
            "number {}..{}, default {} {unit}",
            plain(min),
            plain(max),
            plain(default)
        ),
        ParamKind::Choice { options, default } => {
            format!("one of {}, default {default}", options.join("|"))
        }
        // The transition table has no other kind; the text is only a floor for a future one.
        _ => "a value".to_string(),
    }
}

fn early_issue(
    code: &'static str,
    path: String,
    transition: &str,
    message: String,
    hint: Option<String>,
) -> Issue {
    let mut details = json!({ "transitionId": transition });
    if let Some(hint) = hint {
        details["hint"] = json!(hint);
    }
    Issue::new(code, path, message).with_details(details)
}

/// Step 2 for a known transition effect: the parameters, in document order.
fn read_params(
    spec: &'static EffectSpec,
    raw: Option<&Map<String, Value>>,
    path: &str,
    transition: &str,
    early: &mut Vec<Issue>,
) -> Vec<FxValue> {
    let mut values: Vec<FxValue> = spec
        .params
        .iter()
        .map(|param| param.kind.default_value())
        .collect();
    let Some(raw) = raw else {
        return values;
    };
    let known = spec
        .params
        .iter()
        .map(|param| param.id)
        .collect::<Vec<_>>()
        .join(", ");
    for (name, value) in raw {
        let at = child(path, name);
        let Some((index, param)) = spec.param(name) else {
            early.push(early_issue(
                UNKNOWN_PARAM,
                at,
                transition,
                format!(
                    "Transition {transition}: {} has no parameter {name}",
                    spec.id
                ),
                Some(format!("Parameters of {}: {known}.", spec.id)),
            ));
            continue;
        };
        match parse_param(param, value) {
            Ok(parsed) => values[index] = parsed,
            Err((code, _)) => {
                let problem = if code == PARAM_TYPE {
                    format!("Parameter {name} has the wrong type")
                } else {
                    let shown = match value {
                        Value::Number(number) => spelled(number),
                        Value::String(text) => format!("\"{text}\""),
                        other => other.to_string(),
                    };
                    debug_assert_eq!(code, PARAM_OUT_OF_RANGE);
                    format!("Parameter {name} is {shown}, outside what it accepts")
                };
                early.push(early_issue(
                    code,
                    at,
                    transition,
                    format!("Transition {transition}: {problem}"),
                    Some(format!("{name}: {}.", describe(&param.kind))),
                ));
            }
        }
    }
    values
}

fn read_seed(obj: &Obj<'_>) -> Option<Option<u32>> {
    let seed = obj.opt_strict_int("seed", Bound::NonNegative)?;
    let Some(seed) = seed else {
        return Some(None);
    };
    match u32::try_from(seed) {
        Ok(seed) => Some(Some(seed)),
        Err(_) => {
            obj.sink_issue(
                INVALID_VALUE,
                "seed",
                "expected a seed from 0 to 4294967295",
            );
            None
        }
    }
}

/// `params`: absent, or an object. `Some(None)` is absent; `None` is a wrong type, reported.
fn params_object<'a>(obj: &Obj<'a>, sink: &Sink) -> Option<Option<&'a Map<String, Value>>> {
    match obj.deferred("params") {
        None => Some(None),
        Some(Value::Object(map)) => Some(Some(map)),
        Some(other) => {
            sink.issue(
                INVALID_TYPE,
                &obj.field_path("params"),
                format!("expected object, got {}", kind_of(other)),
            );
            None
        }
    }
}

fn read_one(obj: &Obj<'_>, index: usize, sink: &Sink) -> Option<TransitionDraft> {
    // Step 1: structure and types.
    let id = obj.id("id");
    if let Some(id) = id
        && let Some((code, message)) = id_problem(id)
    {
        sink.issue(code, &obj.field_path("id"), message);
    }
    let effect_name = obj.id("effect");
    let from = obj.id("fromShotId");
    let to = obj.id("toShotId");
    let duration = obj.strict_int("durationFrames", Bound::Any);
    let alignment = obj.text("alignment");
    let handles = obj.text("handles");
    let easing = obj.text("easing");
    let params = params_object(obj, sink);
    // Keys on a transition are entered nowhere yet, not even as `{}`.
    let keys = obj.deferred("keys");
    if keys.is_some() {
        obj.sink_issue(
            FX_TRANSITION_KEYS_NOT_YET,
            "keys",
            "keys on a transition are not implemented yet; leave `keys` out",
        );
    }
    let seed = read_seed(obj);
    let hidden = obj.opt_true("hidden");
    let (id, effect_name, from, to, duration) = (id?, effect_name?, from?, to?, duration?);
    let (alignment, handles, easing, params, seed, hidden) =
        (alignment?, handles?, easing?, params?, seed?, hidden?);
    if id_problem(id).is_some() || keys.is_some() {
        return None;
    }

    // Steps 2 and 3, kept for the explain stage.
    let mut early: Vec<Issue> = Vec::new();
    let effect_path = obj.field_path("effect");
    let spec = match (transition_effect(effect_name), effect(effect_name)) {
        (Some(spec), _) => Some(spec),
        (None, Some(look)) => {
            early.push(early_issue(
                WRONG_FAMILY,
                effect_path,
                id,
                format!(
                    "transition {id}: {effect_name} is a {} effect, not a transition",
                    look.family
                ),
                Some(
                    "A transition takes an effect of the transition family; a look goes on a fx track as a clip."
                        .to_string(),
                ),
            ));
            None
        }
        (None, None) => {
            early.push(early_issue(
                UNKNOWN_EFFECT,
                effect_path,
                id,
                format!("transition {id}: there is no effect {effect_name}"),
                None,
            ));
            None
        }
    };
    let values =
        spec.map(|spec| read_params(spec, params, &obj.field_path("params"), id, &mut early));
    let parsed_easing = FxEasing::parse(easing);
    if parsed_easing.is_none() {
        early.push(early_issue(
            UNKNOWN_KEYFRAME_EASING,
            obj.field_path("easing"),
            id,
            format!("Transition {id} easing {easing} is not supported"),
            Some(format!("Use {}.", FX_EASINGS.join(", "))),
        ));
    }
    let parsed_alignment = Alignment::parse(alignment);
    if parsed_alignment.is_none() {
        early.push(early_issue(
            INVALID_VALUE,
            obj.field_path("alignment"),
            id,
            format!(
                "`{alignment}` is not one of: {}",
                Alignment::NAMES.join(", ")
            ),
            None,
        ));
    }
    let parsed_handles = Handles::parse(handles);
    if parsed_handles.is_none() {
        early.push(early_issue(
            INVALID_VALUE,
            obj.field_path("handles"),
            id,
            format!("`{handles}` is not one of: {}", Handles::NAMES.join(", ")),
            None,
        ));
    }
    let resolved = match (early.is_empty(), spec, values, parsed_easing) {
        (true, Some(effect), Some(params), Some(easing)) => Some(Resolved {
            effect,
            easing,
            params,
            seed,
            hidden,
        }),
        _ => None,
    };
    Some(TransitionDraft {
        index,
        id: id.to_string(),
        from_shot_id: from.to_string(),
        to_shot_id: to.to_string(),
        duration_frames: duration,
        alignment: parsed_alignment,
        handles: parsed_handles,
        early,
        resolved,
    })
}

/// Reads the list. The count is judged first, against the cuts of the edit, so an oversized list
/// costs one comparison and not a pass over its entries (`FX_LIMIT`, T28).
pub(super) fn read_transitions(
    array: &Arr<'_>,
    path: &str,
    shot_count: Option<usize>,
    sink: &Sink,
) -> Option<Vec<TransitionDraft>> {
    if let Some(shots) = shot_count {
        let cuts = shots.saturating_sub(1);
        if array.len() > cuts {
            sink.issue(
                FX_LIMIT,
                path,
                format!(
                    "{} transitions in an edit of {shots} shot(s); it has {cuts} cut(s), one transition each",
                    array.len()
                ),
            );
            return None;
        }
    }
    array.objects(|index, obj| read_one(obj, index, sink))
}
