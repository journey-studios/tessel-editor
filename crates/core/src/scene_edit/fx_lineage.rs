//! Which ops of a batch touched the entity the candidate holds (BL082.4.29, S2-C0a).
//!
//! An id is not an identity: a batch can create `x`, remove it and create another `x`, and the
//! first two ops never touched the `x` that the candidate has. An importer problem on that
//! entity must name the ops that did (the third), not every op that spelled the id. Each
//! creation (`add_clip`, or a `set_transition` that wrote a new cut) starts a new generation of its
//! `(kind, id)`; a removal ends it; only the ops of the generation alive at the end of the batch
//! keep a pointer into the candidate. The rule is private to the edit pipeline: no document
//! field, no public type, no change to the answers of an accepted batch.

use std::collections::HashMap;

use super::FxChange;

/// `(is a transition, id)`: a clip and a transition never share an id (the layer's ids are
/// unique), but they may take it one after the other in one batch, and then they are two entities.
type Entity<'a> = (bool, &'a str);

fn entity_of(change: &FxChange) -> Entity<'_> {
    match &change.transition {
        Some(id) => (true, id.as_str()),
        None => (false, change.clip.as_str()),
    }
}

/// Whether the change made the entity: the action says so, not the shape of `before`/`after`
/// (a key op also goes from nothing to something, on a clip that was already there). A
/// `set_transition` that updated a cut in place has a `before`.
fn creates(change: &FxChange) -> bool {
    match change.action {
        "add_clip" => true,
        "set_transition" => change.before.is_none(),
        _ => false,
    }
}

/// For each change, whether it touched the entity of its id that is still in the candidate: the
/// last generation of the id, alive at the end of the batch. A no-op change touched nothing.
pub(super) fn touched_the_survivor(changes: &[FxChange]) -> Vec<bool> {
    // The generation each change belongs to, in order. An id the batch never creates is the
    // document's own (generation 0).
    let mut current: HashMap<Entity<'_>, (u32, bool)> = HashMap::new();
    let mut generations = Vec::with_capacity(changes.len());
    for change in changes {
        if change.noop {
            generations.push(None);
            continue;
        }
        let entry = current.entry(entity_of(change)).or_insert((0, true));
        if creates(change) {
            *entry = (entry.0 + 1, true);
        }
        generations.push(Some(entry.0));
        if change.action == "remove_clip" || change.action == "remove_transition" {
            entry.1 = false;
        }
    }
    changes
        .iter()
        .zip(generations)
        .map(|(change, generation)| {
            generation.is_some_and(|generation| {
                current.get(&entity_of(change)) == Some(&(generation, true))
            })
        })
        .collect()
}
