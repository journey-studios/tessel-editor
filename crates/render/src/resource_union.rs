//! The host's view of the resources of a plan with one or two decks (BL082.4.4, slice H).
//!
//! The core owns the union ([`ScenePlan::resource_union`]): by `assetId`, ascending, with every use
//! of a resource qualified by its deck. The four host loaders (`SceneResourceMap::from_root`,
//! `SceneSlots::prepare`, `scene_operation::approved_map`, `scene_export::resource_map`) walk that
//! union instead of `plan.resources`, so an asset only the neighbour deck draws is not lost. This
//! module adds nothing to the rules: it reads the union and says, in an error, which resource and
//! which deck a failure belongs to.
//!
//! The context is additive, `details.resource`:
//!
//! ```json
//! { "asset_id": "sky", "used_by": [{ "deck": "primary", "node_id": "a" }, { "deck": "other", "node_id": "b" }] }
//! ```
//!
//! and the message of the error ends with `[asset `sky` used by: primary deck node `a`, other deck
//! node `b`]`, because a job error used to keep its message and little else. Contract:
//! `docs/migration/p4b-s2-host-resource-plan.md`.

use serde_json::{Value, json};
use tessel_editor_core::CommandError;
use tessel_editor_core::backend::BackendError;
use tessel_editor_core::scene_plan::{DeckRole, DeckUse, PlanError, ScenePlan, UnionResource};

use crate::scene::plan_error;

/// The resources of both decks in the core's order (ascending `assetId`), by value.
pub(crate) fn union_of(plan: &ScenePlan) -> Result<Vec<UnionResource>, PlanError> {
    Ok(plan.resource_union()?.into_values().collect())
}

fn use_json(usage: &DeckUse) -> Value {
    json!({ "deck": usage.deck.name(), "node_id": usage.node_id })
}

fn context(asset_id: &str, uses: &[DeckUse]) -> Value {
    json!({ "asset_id": asset_id, "used_by": uses.iter().map(use_json).collect::<Vec<_>>() })
}

/// `{ "asset_id", "used_by": [{ "deck", "node_id" }] }` of one union resource.
pub(crate) fn resource_context(resource: &UnionResource) -> Value {
    context(&resource.asset_id, &resource.used_by)
}

fn uses_text(uses: &[DeckUse]) -> String {
    uses.iter()
        .map(|usage| format!("{} deck node `{}`", usage.deck.name(), usage.node_id))
        .collect::<Vec<_>>()
        .join(", ")
}

fn suffix(asset_id: &str, uses: &[DeckUse]) -> String {
    format!(" [asset `{asset_id}` used by: {}]", uses_text(uses))
}

/// A resource-table pointer names that deck's entry, including errors in one of its fields.
/// Only an error equal to the core union's own refusal is a two-deck metadata conflict;
/// structural failures name the affected deck alone. Uses always come from the resource tables.
fn union_failure_context(plan: &ScenePlan, error: &PlanError) -> Option<(Value, String)> {
    let PlanError::Invalid { pointer, .. } = error else {
        return None;
    };
    let (deck, role, tail) =
        if let Some(tail) = pointer.strip_prefix("/transition/other/resources/") {
            (&plan.transition.as_ref()?.other, DeckRole::Other, tail)
        } else {
            (
                plan,
                DeckRole::Primary,
                pointer.strip_prefix("/resources/")?,
            )
        };
    let position: usize = tail.split('/').next()?.parse().ok()?;
    let resource = deck.resources.get(position)?;
    let uses_of = |deck: &ScenePlan, role| -> Vec<DeckUse> {
        deck.resource(&resource.asset_id)
            .into_iter()
            .flat_map(|entry| &entry.used_by)
            .map(|node_id| DeckUse {
                deck: role,
                node_id: node_id.clone(),
            })
            .collect()
    };
    let uses = if plan.resource_union().err().as_ref() == Some(error) {
        let mut uses = uses_of(plan, DeckRole::Primary);
        uses.extend(uses_of(&plan.transition.as_ref()?.other, DeckRole::Other));
        uses
    } else {
        uses_of(deck, role)
    };
    Some((
        context(&resource.asset_id, &uses),
        suffix(&resource.asset_id, &uses),
    ))
}

fn merged(details: Option<Value>, resource: Value) -> Value {
    match details {
        None | Some(Value::Null) => json!({ "resource": resource }),
        Some(Value::Object(mut map)) => {
            map.insert("resource".to_string(), resource);
            Value::Object(map)
        }
        Some(other) => json!({ "detail": other, "resource": resource }),
    }
}

/// The union's own refusal as a `BackendError` (the bridge's `plan_error`), attributed when it
/// names a resource.
pub(crate) fn union_failure(plan: &ScenePlan, error: &PlanError) -> BackendError {
    let converted = plan_error(error);
    match union_failure_context(plan, error) {
        Some((resource, tail)) => BackendError {
            message: format!("{}{tail}", converted.message),
            details: Some(merged(converted.details, resource)),
            ..converted
        },
        None => converted,
    }
}

/// The union's own refusal as a `CommandError` (the core's `From<PlanError>`), attributed when it
/// names a resource.
pub(crate) fn union_failure_command(plan: &ScenePlan, error: &PlanError) -> CommandError {
    let converted = CommandError::from(error.clone());
    match union_failure_context(plan, error) {
        Some((resource, tail)) => CommandError {
            message: format!("{}{tail}", converted.message),
            details: Some(Box::new(merged(
                converted.details.map(|boxed| *boxed),
                resource,
            ))),
            ..converted
        },
        None => converted,
    }
}
/// Attributes `error` to `resource`: its code and every existing detail stay.
pub(crate) fn attribute_backend(error: BackendError, resource: &UnionResource) -> BackendError {
    BackendError {
        message: format!(
            "{}{}",
            error.message,
            suffix(&resource.asset_id, &resource.used_by)
        ),
        details: Some(merged(error.details, resource_context(resource))),
        ..error
    }
}

/// Attributes `error` to `resource`: its code, target and every existing detail stay.
pub(crate) fn attribute_command(error: CommandError, resource: &UnionResource) -> CommandError {
    CommandError {
        message: format!(
            "{}{}",
            error.message,
            suffix(&resource.asset_id, &resource.used_by)
        ),
        details: Some(Box::new(merged(
            error.details.map(|boxed| *boxed),
            resource_context(resource),
        ))),
        ..error
    }
}
