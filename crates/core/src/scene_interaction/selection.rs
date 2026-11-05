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

/// Entities the shared summary lists (the count is always the whole selection).
pub const SUMMARY_LIMIT: usize = 64;

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
            let Some(reason) = missing_reason(book, selected) else {
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

    /// The selection as the shared summary prints it. Read-only: an entry the project no longer
    /// holds is marked `stale` (with the reason `prune` would give) instead of being removed, so
    /// asking for the summary never changes the selection. The list is cut at
    /// [`SUMMARY_LIMIT`]: a summary is polled, and `scene.selection` has the whole list.
    pub fn summary_json(&self, book: &SceneBook) -> Value {
        let entities: Vec<Value> = self
            .entries
            .iter()
            .take(SUMMARY_LIMIT)
            .map(|selected| {
                let mut value = selected.to_json();
                if let Some(reason) = missing_reason(book, selected) {
                    value["stale"] = json!(reason);
                }
                value
            })
            .collect();
        json!({
            "count": self.entries.len(),
            "entities": entities,
            "truncated": self.entries.len() > SUMMARY_LIMIT,
            "dropped": self.dropped.iter().take(SUMMARY_LIMIT).map(Dropped::to_json)
                .collect::<Vec<_>>(),
            "dropped_count": self.dropped.len(),
        })
    }
}

/// Why `selected` no longer names something in the project: `scene_detached` (the attached scene
/// is gone) or `entity_missing` (the scene is there, the entity is not). `None` while it is live.
fn missing_reason(book: &SceneBook, selected: &Selected) -> Option<&'static str> {
    match book.get(&selected.scene) {
        None => Some("scene_detached"),
        Some(entry) if !exists(entry.scene.bundle(), &selected.entity) => Some("entity_missing"),
        Some(_) => None,
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

    fn book_with(bundle: &str) -> SceneBook {
        use crate::directed_scene::DirectedScene;
        use crate::scene_project::AttachedScene;
        let mut book = SceneBook::default();
        book.insert(AttachedScene {
            id: "street".to_string(),
            name: "Street".to_string(),
            scene: std::sync::Arc::new(DirectedScene::import_str(bundle).unwrap()),
            extra: Default::default(),
        })
        .unwrap();
        book
    }

    const MINIMAL: &str = include_str!(
        "../../../../docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json"
    );
    const RIG: &str = include_str!(
        "../../../../docs/migration/fixtures/bundles/valid-graph-sockets-performance.bundle.json"
    );

    #[test]
    fn the_summary_marks_what_went_stale_without_pruning_it() {
        // `perf-1` is a performance track of the rig bundle only.
        let instance = Entity::new(EntityKind::PerformanceTrack, "perf-1");
        let book = book_with(RIG);
        let mut selection = Selection::default();
        selection
            .apply(Some("street"), vec![instance.clone()], SelectMode::Replace)
            .unwrap();
        let live = selection.summary_json(&book);
        assert_eq!(live["count"], 1);
        assert!(live["entities"][0].get("stale").is_none());

        // A scene that is there without the entity (another bundle under the same ID).
        let replaced = book_with(MINIMAL);
        let missing = selection.summary_json(&replaced);
        assert_eq!(missing["entities"][0]["stale"], "entity_missing");

        // The scene is gone from the project: the entry is reported, not removed.
        let empty = SceneBook::default();
        let stale = selection.summary_json(&empty);
        assert_eq!(stale["entities"][0]["stale"], "scene_detached");
        assert_eq!(stale["count"], 1);
        assert!(
            selection.contains("street", &instance),
            "reading is not pruning"
        );

        // `prune` agrees with what the summary reported, and the summary then remembers it.
        let dropped = selection.prune(&empty);
        assert_eq!(dropped[0].reason, "scene_detached");
        let after = selection.summary_json(&empty);
        assert_eq!(after["count"], 0);
        assert_eq!(after["dropped"][0]["reason"], "scene_detached");
        assert_eq!(after["dropped_count"], 1);
    }

    #[test]
    fn the_summary_list_is_bounded_but_the_count_is_not() {
        let book = book_with(MINIMAL);
        let mut selection = Selection::default();
        let shots = (0..SUMMARY_LIMIT + 5)
            .map(|n| Entity::new(EntityKind::Shot, &format!("shot-{n}")))
            .collect();
        selection
            .apply(Some("street"), shots, SelectMode::Replace)
            .unwrap();
        let summary = selection.summary_json(&book);
        assert_eq!(summary["count"], SUMMARY_LIMIT + 5);
        assert_eq!(summary["entities"].as_array().unwrap().len(), SUMMARY_LIMIT);
        assert_eq!(summary["truncated"], true);
    }
}
