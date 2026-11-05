//! The selection of directed-scene entities: a list of `(attached scene, entity)` pairs.
//!
//! It is presentation state, like the playhead or the selected clip: it is not part of the
//! project document, not part of an undo snapshot and not a precondition of any edit (`scene.edit`
//! names its targets by ID and never reads it). What it does guarantee is that it never keeps
//! naming something that is gone: `prune` drops what no longer exists and says what it dropped.

use crate::directed_scene::Entity;
use crate::scene_project::SceneBook;
use crate::state::CommandError;

use super::address::{entity_json, exists};
use super::command::SelectMode;

use serde_json::{Value, json};

/// WHY a bound: a selection is checked entry by entry on every change, and a script line may
/// list as many entities as its text allows.
pub const MAX_SELECTION: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selected {
    /// ID of the attached scene (the project-level one).
    pub scene: String,
    pub entity: Entity,
}

impl Selected {
    pub fn to_json(&self) -> Value {
        let mut value = entity_json(&self.entity);
        value["scene"] = json!(self.scene);
        value
    }
}

/// An entry `prune` removed, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dropped {
    pub selected: Selected,
    /// `scene_detached` (the attached scene is gone) or `entity_missing` (the scene is there,
    /// the entity is not: an undo or a replaced scene took it away).
    pub reason: &'static str,
}

impl Dropped {
    pub fn to_json(&self) -> Value {
        let mut value = self.selected.to_json();
        value["reason"] = json!(self.reason);
        value
    }
}

#[derive(Debug, Clone, Default)]
pub struct Selection {
    entries: Vec<Selected>,
    /// What `prune` removed since the selection was last set by a command, so a caller that
    /// was not watching (an undo happened in between) can still learn why an entity left.
    dropped: Vec<Dropped>,
}

impl Selection {
    pub fn entries(&self) -> &[Selected] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn dropped(&self) -> &[Dropped] {
        &self.dropped
    }

    pub fn contains(&self, scene: &str, entity: &Entity) -> bool {
        self.entries
            .iter()
            .any(|selected| selected.scene == scene && &selected.entity == entity)
    }

    /// Entities selected inside one attached scene, in selection order.
    pub fn entities_of<'a>(&'a self, scene: &'a str) -> impl Iterator<Item = &'a Entity> + 'a {
        self.entries
            .iter()
            .filter(move |selected| selected.scene == scene)
            .map(|selected| &selected.entity)
    }

    /// Applies `mode` with `entities` (already checked against the scene by the caller).
    /// Returns whether the selection changed. Atomic: on error nothing changed.
    pub(crate) fn apply(
        &mut self,
        scene: Option<&str>,
        entities: Vec<Entity>,
        mode: SelectMode,
    ) -> Result<bool, CommandError> {
        let mut next: Vec<Selected> = match (mode, scene) {
            (SelectMode::Replace, _) => Vec::new(),
            (SelectMode::Clear, None) => Vec::new(),
            (SelectMode::Clear, Some(scene)) => self
                .entries
                .iter()
                .filter(|selected| selected.scene != scene)
                .cloned()
                .collect(),
            _ => self.entries.clone(),
        };
        // Only `Clear` may come without a scene; the caller checks it, so this is a guard.
        if let Some(scene) = scene {
            for entity in entities {
                let selected = Selected {
                    scene: scene.to_string(),
                    entity,
                };
                let position = next.iter().position(|entry| *entry == selected);
                match (mode, position) {
                    (SelectMode::Replace | SelectMode::Add, None) => next.push(selected),
                    (SelectMode::Remove | SelectMode::Toggle, Some(index)) => {
                        next.remove(index);
                    }
                    (SelectMode::Toggle, None) => next.push(selected),
                    _ => {}
                }
            }
        }
        if next.len() > MAX_SELECTION {
            return Err(CommandError::new(
                "selection_limit",
                format!("A selection holds at most {MAX_SELECTION} entities"),
            ));
        }
        let changed = next != self.entries;
        self.entries = next;
        self.dropped.clear();
        Ok(changed)
    }

    /// Removes what the project no longer holds and returns it (also kept in `dropped`).
    pub(crate) fn prune(&mut self, book: &SceneBook) -> Vec<Dropped> {
        let mut removed = Vec::new();
        self.entries.retain(|selected| {
            let reason = match book.get(&selected.scene) {
                None => Some("scene_detached"),
                Some(entry) if !exists(entry.scene.bundle(), &selected.entity) => {
                    Some("entity_missing")
                }
                Some(_) => None,
            };
            let Some(reason) = reason else {
                return true;
            };
            removed.push(Dropped {
                selected: selected.clone(),
                reason,
            });
            false
        });
        self.dropped.extend(removed.iter().cloned());
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::directed_scene::EntityKind;

    fn node(id: &str) -> Entity {
        Entity::node("def", id)
    }

    #[test]
    fn modes_replace_add_remove_toggle_and_clear() {
        let mut selection = Selection::default();
        assert!(
            selection
                .apply(
                    Some("s"),
                    vec![node("a"), node("b"), node("a")],
                    SelectMode::Replace
                )
                .unwrap()
        );
        assert_eq!(selection.entries().len(), 2, "repeats collapse");

        assert!(
            !selection
                .apply(Some("s"), vec![node("a")], SelectMode::Add)
                .unwrap(),
            "adding what is there changes nothing"
        );
        assert!(
            selection
                .apply(Some("s"), vec![node("c")], SelectMode::Toggle)
                .unwrap()
        );
        assert!(selection.contains("s", &node("c")));
        selection
            .apply(Some("s"), vec![node("a"), node("c")], SelectMode::Toggle)
            .unwrap();
        assert!(!selection.contains("s", &node("a")));
        assert!(!selection.contains("s", &node("c")));
        selection
            .apply(
                Some("s"),
                vec![node("b"), node("ghost")],
                SelectMode::Remove,
            )
            .unwrap();
        assert!(
            selection.is_empty(),
            "removing an absent entry is not an error"
        );

        selection
            .apply(Some("one"), vec![node("a")], SelectMode::Replace)
            .unwrap();
        selection
            .apply(Some("two"), vec![node("a")], SelectMode::Add)
            .unwrap();
        selection
            .apply(Some("one"), vec![], SelectMode::Clear)
            .unwrap();
        assert_eq!(selection.entries().len(), 1);
        assert_eq!(selection.entries()[0].scene, "two");
        assert!(selection.apply(None, vec![], SelectMode::Clear).unwrap());
        assert!(selection.is_empty());
    }

    #[test]
    fn an_oversized_selection_is_refused_and_leaves_the_old_one() {
        let mut selection = Selection::default();
        selection
            .apply(Some("s"), vec![node("keep")], SelectMode::Replace)
            .unwrap();
        let many = (0..=MAX_SELECTION)
            .map(|n| Entity::new(EntityKind::Shot, &format!("shot-{n}")))
            .collect();
        let error = selection
            .apply(Some("s"), many, SelectMode::Add)
            .unwrap_err();
        assert_eq!(error.code, "selection_limit");
        assert!(selection.contains("s", &node("keep")));
        assert_eq!(selection.entries().len(), 1);
    }
}
