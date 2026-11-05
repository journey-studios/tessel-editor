//! F036: directed scenes (`render-bundle` 1.0) attached to a project, and the programmatic access
//! to them: attach by ID, list, query the catalog, properties and camera, edit in batches with
//! dry-run, undo/redo and save/reopen.
//!
//! This module is the adapter between three things that already exist and does not add rules of
//! its own to any of them:
//!
//! * [`DirectedScene`](crate::directed_scene::DirectedScene) (BB01/BB02/BB03) validates and
//!   evaluates a bundle;
//! * [`scene_edit`](crate::scene_edit) (BB04) edits it atomically by ID and already runs the
//!   importer, the structural join gate and the temporal gate on every candidate;
//! * [`EditorState`](crate::EditorState) owns the history and the project document.
//!
//! What lives here is the glue the editor needs: the [`SceneBook`] a project carries, the
//! [`SceneCommand`] vocabulary (`scene.*` operations, parsed once in `ops::parse_op` so the
//! headless CLI and `ui op` share it), the versioned `scenes` block of `.tessel.json`, and the
//! declaration of what a scene does *not* do yet ([`scope_json`]).
//!
//! Scope, deliberately narrow (see `docs/features/scene-project.md`):
//!
//! * A scene is **interchange data kept in the project**. Nothing renders, plays, exports or runs
//!   it; it is never a `tessel-scene@1` asset (F030) and never reaches the viewport runtime.
//! * Resources the bundle names by relative path are not resolved, copied or packaged (BB05/BB06).
//!   Package and project-import therefore refuse to pretend otherwise (`crates/media` guards).
//! * `scene.camera` answers the camera only; `scene.evaluate` (F039) answers the whole frame,
//!   nodes included, and says what the scene declares and nothing evaluates.
//! * Selection, the entity tree and the begin/update/commit/cancel gesture cycle (F039) live in
//!   `scene_interaction` and run in `interaction.rs`. Their state is session state, not document.
//!
//! Mutation goes through `EditorState::execute(Command::Scene(..))`, the same door as every other
//! edit, so one command is one undo entry and a rejected, dry-run or no-op command leaves the
//! project and its history untouched. Reading goes through the same door (`history_label` is
//! `None`) so a caller has a single entry point.

use std::sync::Arc;

use serde_json::{Map, Value};

use crate::directed_scene::DirectedScene;
use crate::id::MAX_ID_LEN;
use crate::state::CommandError;

mod command;
mod interaction;
mod query;
mod run;
mod scope;
mod store;

pub use command::{AdmittedScene, CameraAt, SceneCommand, SceneSource};
pub use run::admit;
pub use scope::{deferred_in_use, scope_json};
pub use store::STORE_VERSION;

/// Scenes a project may hold. WHY a bound: each bundle may be tens of megabytes (the importer's
/// `Limits::max_bytes`), and a document opened from disk or a script in a loop must not be able
/// to grow the project (and every undo snapshot that shares it) without limit.
pub const MAX_SCENES: usize = 64;

/// Total JSON bytes of every attached bundle. WHY: `persist::load` refuses a document above
/// `MAX_DOCUMENT_BYTES` (256 MiB) and `persist::save` does not check, so without this bound a
/// project could exceed the document budget. This compact-data cap is enforced on attach,
/// open and edits; persistence separately bounds the actual pretty-printed document.
pub const MAX_SCENE_BYTES: usize = 128 * 1024 * 1024;

/// A bundle attached to the project under a stable ID.
#[derive(Debug, Clone, PartialEq)]
pub struct AttachedScene {
    /// Chosen by the caller (or generated as `scene-<n>`): never derived from a selection, a
    /// window or the bundle's own `bundleId`, so scripts and saved documents keep addressing it.
    pub id: String,
    /// Display label; the bundle's `bundleId` unless the caller gave one.
    pub name: String,
    /// WHY `Arc`: undo snapshots, `AddClip`'s rollback copy and package plans clone the whole
    /// `EditorState`; the bundle's JSON tree is shared instead of copied each time. An edit builds
    /// a new `DirectedScene` and swaps the pointer, so a snapshot keeps the old one intact.
    pub scene: Arc<DirectedScene>,
    /// Keys of the stored entry that this editor does not know. A newer editor may add some; they
    /// survive an open/save here so the older editor does not erase what it cannot read.
    pub(crate) extra: Map<String, Value>,
}

/// The scenes of a project, in attach order (the order is part of the saved document).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SceneBook {
    scenes: Vec<AttachedScene>,
    // Compact byte counts aligned with scenes; unchanged bundles are never reserialized.
    scene_bytes: Vec<usize>,
    /// Keys of the stored `scenes` block that this editor does not know (kept on save).
    extra: Map<String, Value>,
}

/// Scene IDs share the alphabet of [`Id`](crate::id::Id) so they can appear in semantic targets
/// (`scene:<id>`) without escaping. They are plain `String`s and are never interned: an `Id` lives
/// until the process ends, and a scene ID may come from an arbitrary document or script line.
pub fn check_scene_id(raw: &str) -> Result<(), CommandError> {
    let problem = if raw.is_empty() {
        Some("an ID cannot be empty".to_string())
    } else if raw.len() > MAX_ID_LEN {
        Some(format!(
            "an ID has at most {MAX_ID_LEN} bytes, got {}",
            raw.len()
        ))
    } else {
        raw.chars()
            .find(|ch| !(ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.')))
            .map(|ch| {
                format!("`{ch}` is not allowed in an ID (use letters, digits, `-`, `_` or `.`)")
            })
    };
    match problem {
        None => Ok(()),
        Some(message) => Err(CommandError::new("invalid_id", message).target(raw)),
    }
}

/// Counts compact JSON without allocating a second copy of every attached bundle.
fn serialized_len(scene: &DirectedScene) -> usize {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.saturating_add(bytes.len());
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, scene.document()).map_or(usize::MAX, |()| counter.0)
}

impl SceneBook {
    pub fn is_empty(&self) -> bool {
        self.scenes.is_empty()
    }

    /// Includes unknown block metadata, even after the last scene is removed.
    pub fn has_document_data(&self) -> bool {
        !self.scenes.is_empty() || !self.extra.is_empty()
    }

    pub fn len(&self) -> usize {
        self.scenes.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &AttachedScene> {
        self.scenes.iter()
    }

    pub fn get(&self, id: &str) -> Option<&AttachedScene> {
        self.scenes.iter().find(|entry| entry.id == id)
    }

    pub fn ids(&self) -> Vec<&str> {
        self.scenes.iter().map(|entry| entry.id.as_str()).collect()
    }

    /// Files each scene names by relative path (images, the audio mix), for the scenes that name
    /// any. They are data inside the document; nothing resolves, hashes or copies them (BB05/06),
    /// so `crates/media` reads this to refuse calling a package portable while they are missing
    /// from it.
    pub fn resource_paths(&self) -> Vec<(&str, Vec<&str>)> {
        self.scenes
            .iter()
            .filter_map(|entry| {
                let bundle = entry.scene.bundle();
                let mut paths: Vec<&str> = bundle
                    .assets
                    .iter()
                    .map(|asset| asset.relative_path.as_str())
                    .collect();
                paths.extend(
                    bundle
                        .audio
                        .iter()
                        .map(|mix| mix.mix_relative_path.as_str()),
                );
                (!paths.is_empty()).then_some((entry.id.as_str(), paths))
            })
            .collect()
    }

    /// The gate `scene.attach` runs before it reads anything: a well-formed, free ID (or room for a
    /// generated one) and a name that is not blank. Public so an adapter that reads the bundle on a
    /// job thread can refuse these without first spending that work (the command checks again when
    /// the result is applied, because the project may have changed meanwhile).
    pub fn check_attach(&self, id: Option<&str>, name: Option<&str>) -> Result<(), CommandError> {
        match id {
            Some(raw) => {
                check_scene_id(raw)?;
                self.check_room(raw)?;
            }
            None => self.check_room(&self.fresh_id())?,
        }
        if name.is_some_and(|raw| raw.trim().is_empty()) {
            return Err(CommandError::invalid_value(
                "A scene name cannot be blank",
                serde_json::json!("a non-empty string"),
            ));
        }
        Ok(())
    }

    /// Whether `id` can be added now: free and under [`MAX_SCENES`]. Checked before the (costly)
    /// import so a refusal does not first parse a large bundle.
    pub(crate) fn check_room(&self, id: &str) -> Result<(), CommandError> {
        if self.get(id).is_some() {
            return Err(CommandError::new(
                "duplicate_id",
                format!("A scene with ID `{id}` is already attached"),
            )
            .target(id));
        }
        if self.scenes.len() >= MAX_SCENES {
            return Err(CommandError::new(
                "scene_limit",
                format!("A project holds at most {MAX_SCENES} scenes"),
            )
            .target(id));
        }
        Ok(())
    }

    fn stored_bytes(&self) -> usize {
        self.scene_bytes.iter().sum()
    }

    pub(crate) fn insert(&mut self, entry: AttachedScene) -> Result<(), CommandError> {
        self.check_room(&entry.id)?;
        let bytes = serialized_len(&entry.scene);
        let total = self.stored_bytes().saturating_add(bytes);
        if total > MAX_SCENE_BYTES {
            return Err(CommandError::new(
                "scenes_too_large",
                format!(
                    "The scenes of a project hold at most {MAX_SCENE_BYTES} bytes of JSON; \
                     attaching `{}` would make it {total}",
                    entry.id
                ),
            )
            .target(entry.id.as_str()));
        }
        self.scenes.push(entry);
        self.scene_bytes.push(bytes);
        Ok(())
    }

    pub(crate) fn remove(&mut self, id: &str) -> Option<AttachedScene> {
        let index = self.scenes.iter().position(|entry| entry.id == id)?;
        self.scene_bytes.remove(index);
        Some(self.scenes.remove(index))
    }

    pub(crate) fn check_replacement_size(
        &self,
        id: &str,
        scene: &DirectedScene,
    ) -> Result<(), CommandError> {
        self.check_replacement_size_with(id, scene, MAX_SCENE_BYTES)
    }

    fn check_replacement_size_with(
        &self,
        id: &str,
        scene: &DirectedScene,
        limit: usize,
    ) -> Result<(), CommandError> {
        let index = self
            .scenes
            .iter()
            .position(|entry| entry.id == id)
            .ok_or_else(|| CommandError::unknown_element("scene", id))?;
        let total = self
            .stored_bytes()
            .saturating_sub(self.scene_bytes[index])
            .saturating_add(serialized_len(scene));
        if total > limit {
            return Err(CommandError::new("scenes_too_large", format!("Editing scene `{id}` would exceed the {limit}-byte scene budget ({total} bytes)")).target(id));
        }
        Ok(())
    }

    /// Swaps only after the replacement fits the same size budget as attach/open.
    pub(crate) fn replace_scene(
        &mut self,
        id: &str,
        scene: DirectedScene,
    ) -> Result<(), CommandError> {
        self.replace_scene_with_limit(id, scene, MAX_SCENE_BYTES)
    }

    fn replace_scene_with_limit(
        &mut self,
        id: &str,
        scene: DirectedScene,
        limit: usize,
    ) -> Result<(), CommandError> {
        self.check_replacement_size_with(id, &scene, limit)?;
        let index = self
            .scenes
            .iter()
            .position(|entry| entry.id == id)
            .ok_or_else(|| CommandError::unknown_element("scene", id))?;
        self.scene_bytes[index] = serialized_len(&scene);
        self.scenes[index].scene = Arc::new(scene);
        Ok(())
    }

    /// `scene-<n>`, the first free one: generated IDs are reproducible for a given project.
    pub(crate) fn fresh_id(&self) -> String {
        (1..)
            .map(|n| format!("scene-{n}"))
            .find(|candidate| self.get(candidate).is_none())
            .unwrap_or_else(|| "scene".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::Id;

    #[test]
    fn scene_ids_accept_exactly_what_entity_ids_accept() {
        let long = "a".repeat(MAX_ID_LEN + 1);
        let exact = "a".repeat(MAX_ID_LEN);
        for raw in [
            "street",
            "scene-1",
            "a.b_c",
            "",
            "has space",
            "colon:no",
            "slash/no",
            "tab\t",
            "é",
            long.as_str(),
            exact.as_str(),
        ] {
            assert_eq!(
                check_scene_id(raw).is_ok(),
                Id::new(raw).is_ok(),
                "`{raw}` must be judged like an entity ID"
            );
        }
    }

    #[test]
    fn a_rejected_id_names_the_offending_value() {
        let error = check_scene_id("bad id").unwrap_err();
        assert_eq!(error.code, "invalid_id");
        assert_eq!(error.target.as_deref(), Some("bad id"));
    }

    #[test]
    fn generated_ids_skip_the_taken_ones() {
        let mut book = SceneBook::default();
        assert_eq!(book.fresh_id(), "scene-1");
        let bundle: Value = serde_json::from_str(include_str!(
            "../../../../docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json"
        ))
        .unwrap();
        let scene = Arc::new(DirectedScene::import_value(bundle).unwrap());
        for id in ["scene-1", "scene-2"] {
            book.insert(AttachedScene {
                id: id.to_string(),
                name: id.to_string(),
                scene: Arc::clone(&scene),
                extra: Map::new(),
            })
            .unwrap();
        }
        assert_eq!(book.fresh_id(), "scene-3");
        assert_eq!(book.ids(), ["scene-1", "scene-2"]);
        let again = book.get("scene-1").unwrap().clone();
        assert_eq!(book.insert(again).unwrap_err().code, "duplicate_id");
    }
}

#[cfg(test)]
mod edit_budget_tests {
    use super::*;

    #[test]
    fn cached_sizes_follow_insert_replace_remove_and_snapshot() {
        let raw: Value = serde_json::from_str(include_str!(
            "../../../../docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json"
        ))
        .unwrap();
        let original = DirectedScene::import_value(raw.clone()).unwrap();
        let mut book = SceneBook::default();
        for id in ["a", "b"] {
            book.insert(AttachedScene {
                id: id.into(),
                name: id.into(),
                scene: Arc::new(original.clone()),
                extra: Map::new(),
            })
            .unwrap();
        }
        let snapshot = book.clone();
        let mut changed = raw;
        changed["x-size-probe"] = Value::String("larger".into());
        let changed = DirectedScene::import_value(changed).unwrap();
        book.replace_scene("a", changed).unwrap();
        assert_eq!(
            book.stored_bytes(),
            book.iter()
                .map(|entry| serialized_len(&entry.scene))
                .sum::<usize>()
        );
        assert_eq!(snapshot.stored_bytes(), 2 * serialized_len(&original));
        book.remove("a").unwrap();
        assert_eq!(book.stored_bytes(), serialized_len(&original));
        book.remove("b").unwrap();
        assert_eq!(book.stored_bytes(), 0);
        assert!(book.scene_bytes.is_empty());
    }

    #[test]
    fn a_replacement_over_budget_is_rejected_before_the_scene_is_changed() {
        let raw: Value = serde_json::from_str(include_str!(
            "../../../../docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json"
        ))
        .unwrap();
        let original = DirectedScene::import_value(raw.clone()).unwrap();
        let mut book = SceneBook::default();
        for id in ["a", "b"] {
            book.insert(AttachedScene {
                id: id.into(),
                name: id.into(),
                scene: Arc::new(original.clone()),
                extra: Map::new(),
            })
            .unwrap();
        }
        let limit = book.stored_bytes();
        let before = book.clone();
        let mut bigger = raw;
        bigger["x-size-probe"] = Value::String("larger".into());
        let bigger = DirectedScene::import_value(bigger).unwrap();
        assert!(
            book.check_replacement_size_with("a", &bigger, limit)
                .is_err()
        );
        assert_eq!(
            book.replace_scene_with_limit("a", bigger, limit)
                .unwrap_err()
                .code,
            "scenes_too_large"
        );
        assert_eq!(book, before);
        assert!(book.replace_scene_with_limit("a", original, limit).is_ok());
    }
}
