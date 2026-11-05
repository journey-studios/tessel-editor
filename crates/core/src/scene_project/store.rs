//! The `scenes` block of `.tessel.json`.
//!
//! ```json
//! "scenes": {
//!   "version": 1,
//!   "items": [ { "id": "street", "name": "Street", "bundle": { ...render-bundle 1.0... } } ]
//! }
//! ```
//!
//! A project containing this block uses project schema v2. Legacy v1 readers reject it rather
//! than discard scenes on save. Projects without scene data still write v1 unchanged.
//!
//! The bundle is stored as the imported JSON tree (`DirectedScene::document`), so fields the
//! importer does not interpret, unknown fields included, come back byte-equal as data. Keys of an
//! entry or of the block that this editor does not know are kept too.

use std::sync::Arc;

use serde_json::{Map, Value, json};

use crate::directed_scene::DirectedScene;
use crate::state::CommandError;

use super::run::admit;
use super::{AttachedScene, SceneBook, check_scene_id};

/// Version of the `scenes` block this editor writes and the newest it reads.
pub const STORE_VERSION: u64 = 1;

const BLOCK_KEYS: [&str; 2] = ["version", "items"];
const ENTRY_KEYS: [&str; 3] = ["id", "name", "bundle"];

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::new("invalid_document", message)
}

/// Keeps the code and the findings of `error` (`invalid_bundle` and its issue list, for one) and
/// says which stored scene they belong to.
fn in_document(id: &str, mut error: CommandError) -> CommandError {
    error.message = format!("Scene `{id}` of the document: {}", error.message);
    error.target(format!("scene:{id}"))
}

fn unknown_keys(object: &Map<String, Value>, known: &[&str]) -> Map<String, Value> {
    object
        .iter()
        .filter(|(key, _)| !known.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn parse_entry(item: &Value) -> Result<AttachedScene, CommandError> {
    let Some(object) = item.as_object() else {
        return Err(invalid("Every stored scene must be an object"));
    };
    let id = object
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("A stored scene needs a string `id`"))?;
    check_scene_id(id)?;
    let bundle = object
        .get("bundle")
        .ok_or_else(|| invalid(format!("Scene `{id}` needs a `bundle`")))?;
    // The same admission as `scene.attach`: a hand-edited document cannot smuggle in a bundle the
    // editor itself would refuse.
    let scene = DirectedScene::import_value(bundle.clone())
        .map_err(CommandError::from)
        .and_then(admit)
        .map_err(|error| in_document(id, error))?;
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .map_or_else(|| scene.bundle().bundle_id.clone(), str::to_string);
    Ok(AttachedScene {
        id: id.to_string(),
        name,
        scene: Arc::new(scene),
        extra: unknown_keys(object, &ENTRY_KEYS),
    })
}

impl SceneBook {
    /// The block to store, `None` for a project that never had a scene (nothing is written).
    pub fn to_json(&self) -> Option<Value> {
        if self.scenes.is_empty() && self.extra.is_empty() {
            return None;
        }
        let items: Vec<Value> = self
            .scenes
            .iter()
            .map(|entry| {
                let mut object = entry.extra.clone();
                object.insert("id".to_string(), json!(entry.id));
                object.insert("name".to_string(), json!(entry.name));
                object.insert("bundle".to_string(), entry.scene.document().clone());
                Value::Object(object)
            })
            .collect();
        let mut block = self.extra.clone();
        block.insert("version".to_string(), json!(STORE_VERSION));
        block.insert("items".to_string(), Value::Array(items));
        Some(Value::Object(block))
    }

    /// Reads the block of a document. Nothing is half built: any bad entry fails the whole read.
    pub fn from_json(block: &Value) -> Result<Self, CommandError> {
        let Some(object) = block.as_object() else {
            return Err(invalid("`scenes` must be an object"));
        };
        let version = object
            .get("version")
            .and_then(Value::as_u64)
            .ok_or_else(|| invalid("`scenes.version` is missing"))?;
        if version > STORE_VERSION {
            return Err(CommandError::new(
                "unsupported_schema",
                format!(
                    "This project stores scenes as version {version}; this editor reads up to \
                     {STORE_VERSION}. Open it with a newer Tessel Editor."
                ),
            )
            .details(json!({ "found": version, "supported": STORE_VERSION })));
        }
        if version == 0 {
            return Err(invalid("`scenes.version` 0 never existed"));
        }
        let items = object
            .get("items")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("`scenes.items` must be an array"))?;
        let mut book = Self::default();
        for item in items {
            book.insert(parse_entry(item)?).map_err(|error| {
                invalid(format!("{}: {}", error.code, error.message))
                    .target(error.target.unwrap_or_default())
            })?;
        }
        book.extra = unknown_keys(object, &BLOCK_KEYS);
        Ok(book)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle() -> Value {
        serde_json::from_str(include_str!(
            "../../../../docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json"
        ))
        .unwrap()
    }

    fn book_with(id: &str) -> SceneBook {
        let mut book = SceneBook::default();
        book.insert(AttachedScene {
            id: id.to_string(),
            name: "Name".to_string(),
            scene: Arc::new(DirectedScene::import_value(bundle()).unwrap()),
            extra: Map::new(),
        })
        .unwrap();
        book
    }

    #[test]
    fn an_empty_book_writes_nothing_so_old_projects_keep_their_bytes() {
        assert_eq!(SceneBook::default().to_json(), None);
    }

    #[test]
    fn the_block_round_trips_bundle_name_and_unknown_keys() {
        let mut raw = bundle();
        raw["x-tool"] = json!({ "kept": [1, 2, 3] });
        let mut book = SceneBook::default();
        let scene = DirectedScene::import_value(raw).unwrap();
        let mut extra = Map::new();
        extra.insert("x-entry".to_string(), json!("kept"));
        book.insert(AttachedScene {
            id: "street".to_string(),
            name: "Street".to_string(),
            scene: Arc::new(scene),
            extra,
        })
        .unwrap();
        book.extra.insert("x-block".to_string(), json!(7));

        let block = book.to_json().unwrap();
        assert_eq!(block["version"], 1);
        assert_eq!(block["x-block"], 7);
        assert_eq!(block["items"][0]["x-entry"], "kept");
        assert_eq!(
            block["items"][0]["bundle"]["x-tool"]["kept"],
            json!([1, 2, 3])
        );

        let reread = SceneBook::from_json(&block).unwrap();
        assert_eq!(reread, book);
        assert_eq!(reread.to_json(), Some(block));
    }

    #[test]
    fn a_block_from_the_future_is_refused_with_a_typed_error() {
        let mut block = book_with("a").to_json().unwrap();
        block["version"] = json!(STORE_VERSION + 1);
        let error = SceneBook::from_json(&block).unwrap_err();
        assert_eq!(error.code, "unsupported_schema");
    }

    #[test]
    fn a_bad_entry_keeps_the_importers_code_and_names_the_scene() {
        let mut block = book_with("a").to_json().unwrap();
        block["items"][0]["bundle"]["schemaVersion"] = json!("9.9");
        let error = SceneBook::from_json(&block).unwrap_err();
        assert_eq!(error.code, "invalid_bundle");
        assert_eq!(error.target.as_deref(), Some("scene:a"));
        let issues = error.details.as_ref().unwrap()["issues"]
            .as_array()
            .unwrap()
            .clone();
        assert!(!issues.is_empty());
    }

    #[test]
    fn malformed_blocks_are_invalid_documents_not_panics() {
        for block in [
            json!([]),
            json!({}),
            json!({ "version": 1 }),
            json!({ "version": 0, "items": [] }),
            json!({ "version": 1, "items": [1] }),
            json!({ "version": 1, "items": [{ "name": "x" }] }),
            json!({ "version": 1, "items": [{ "id": "a b", "bundle": {} }] }),
        ] {
            assert!(SceneBook::from_json(&block).is_err(), "{block}");
        }
    }

    #[test]
    fn duplicate_ids_in_a_document_are_refused() {
        let mut block = book_with("a").to_json().unwrap();
        let again = block["items"][0].clone();
        block["items"].as_array_mut().unwrap().push(again);
        assert!(SceneBook::from_json(&block).is_err());
    }
}
