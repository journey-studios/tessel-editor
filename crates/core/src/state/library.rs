//! Project bins and the typed asset pool. Demonstration items are illustrative; items
//! created through the API carry an [`Asset`] probed from a real file.

use serde_json::{Value, json};

use super::error::CommandError;
use crate::assets::{Asset, Identity, Meta, Source};
use crate::id::Id;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderIcon {
    Project,
    Footage,
    Timelines,
    Audio,
}

impl FolderIcon {
    pub fn label(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Footage => "footage",
            Self::Timelines => "timelines",
            Self::Audio => "audio",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Folder {
    pub id: Id,
    pub label: String,
    pub icon: FolderIcon,
    pub parent: Option<Id>,
    pub expanded: bool,
}

pub use crate::assets::{DemoImage, MediaKind};

#[derive(Debug, Clone, PartialEq)]
pub struct MediaItem {
    pub id: Id,
    pub code: String,
    pub name: String,
    pub meta: String,
    pub kind: MediaKind,
    pub folder: Id,
    pub image: Option<DemoImage>,
    /// Source and probed metadata. `None` for the sequence entry and for demonstration
    /// audio that has no file behind it.
    pub asset: Option<Asset>,
}

#[derive(Debug, Clone)]
pub struct Library {
    pub folders: Vec<Folder>,
    pub items: Vec<MediaItem>,
    /// `None` is the "All media" pseudo-bin.
    pub selected_folder: Option<Id>,
    pub search: String,
}

pub const ROOT_FOLDER: &str = "north-expedition";
/// Root bin of a project created empty.
pub const EMPTY_ROOT_FOLDER: &str = "project";

impl Default for Library {
    /// The demonstration library. Prefer [`Library::demo`] in new code so the intent is explicit.
    fn default() -> Self {
        Self::demo()
    }
}

impl Library {
    /// Bootstrap of the illustrative "North expedition" project: stills with bundled
    /// demo images, a timeline entry and two audio entries without media.
    pub fn demo() -> Self {
        let folder = |id, label: &str, icon, parent| Folder {
            id: Id::lit(id),
            label: label.to_string(),
            icon,
            parent,
            expanded: true,
        };
        let still = |id: &str, code: &str, name: &str, image| MediaItem {
            id: Id::lit(id),
            code: code.to_string(),
            name: name.to_string(),
            meta: "STILL · 1920 × 1080".to_string(),
            kind: MediaKind::Still,
            folder: Id::lit("footage"),
            image: Some(image),
            asset: Some(Asset {
                source: Source::Demo(image),
                meta: Meta {
                    width: Some(1920),
                    height: Some(1080),
                    format: Some("png".into()),
                    ..Meta::default()
                },
                identity: Identity::default(),
                decoded: None,
            }),
        };
        let audio = |id: &str, code: &str, name: &str| MediaItem {
            id: Id::lit(id),
            code: code.to_string(),
            name: name.to_string(),
            meta: "AUDIO · demo waveform".to_string(),
            kind: MediaKind::Audio,
            folder: Id::lit("audio"),
            image: None,
            asset: None,
        };
        let root = Id::lit(ROOT_FOLDER);
        Self {
            folders: vec![
                folder(ROOT_FOLDER, "North expedition", FolderIcon::Project, None),
                folder("footage", "Footage", FolderIcon::Footage, Some(root)),
                folder("timelines", "Timelines", FolderIcon::Timelines, Some(root)),
                folder("audio", "Audio", FolderIcon::Audio, Some(root)),
            ],
            items: vec![
                still("ao01", "AO01", "First light", DemoImage::Highlands),
                still("ao02", "AO02", "Atlantic coast", DemoImage::Coast),
                still("ao03", "AO03", "River detail", DemoImage::Highlands),
                still("ao04", "AO04", "Basalt cliffs", DemoImage::Coast),
                MediaItem {
                    id: Id::lit("master-sequence"),
                    code: "TL01".to_string(),
                    name: "Master sequence".to_string(),
                    meta: "TIMELINE · 24 fps".to_string(),
                    kind: MediaKind::Timeline,
                    folder: Id::lit("timelines"),
                    image: None,
                    asset: None,
                },
                audio("original-score", "AU01", "North / original score"),
                audio("surf-texture", "AU02", "Atlantic / surf texture"),
            ],
            selected_folder: None,
            search: String::new(),
        }
    }

    /// A project library with one root bin and no items.
    pub fn empty(root_label: &str) -> Self {
        Self {
            folders: vec![Folder {
                id: Id::lit(EMPTY_ROOT_FOLDER),
                label: root_label.to_string(),
                icon: FolderIcon::Project,
                parent: None,
                expanded: true,
            }],
            items: Vec::new(),
            selected_folder: None,
            search: String::new(),
        }
    }

    pub fn root(&self) -> Option<&Folder> {
        self.folders.iter().find(|folder| folder.parent.is_none())
    }

    /// Adds an item after checking its ID is free and its bin exists.
    pub fn add_item(&mut self, item: MediaItem) -> Result<(), CommandError> {
        if self.item(item.id).is_some() {
            return Err(CommandError::new(
                "duplicate_id",
                format!("An item with ID `{}` already exists", item.id),
            )
            .target(item.id.as_str()));
        }
        if self.folder(item.folder).is_none() {
            return Err(CommandError::unknown_element("folder", &item.folder));
        }
        self.items.push(item);
        Ok(())
    }

    pub fn folder(&self, id: impl AsRef<str>) -> Option<&Folder> {
        let id = id.as_ref();
        self.folders.iter().find(|folder| folder.id == id)
    }

    pub fn item(&self, id: impl AsRef<str>) -> Option<&MediaItem> {
        let id = id.as_ref();
        self.items.iter().find(|item| item.id == id)
    }

    pub fn children<'a>(&'a self, id: &'a str) -> impl Iterator<Item = &'a Folder> {
        self.folders
            .iter()
            .filter(move |folder| folder.parent.is_some_and(|parent| parent == id))
    }

    pub fn has_children(&self, id: impl AsRef<str>) -> bool {
        self.children(id.as_ref()).next().is_some()
    }

    pub fn depth(&self, id: impl AsRef<str>) -> usize {
        let mut depth = 0;
        let mut current = self.folder(id).and_then(|folder| folder.parent);
        while let Some(parent) = current {
            depth += 1;
            current = self.folder(parent).and_then(|folder| folder.parent);
        }
        depth
    }

    /// A folder is visible when every ancestor is expanded, so collapsing a
    /// group hides its whole subtree rather than only its direct children.
    pub fn is_visible(&self, id: impl AsRef<str>) -> bool {
        let id = id.as_ref();
        let mut current = self.folder(id).and_then(|folder| folder.parent);
        while let Some(parent) = current {
            let Some(parent_folder) = self.folder(parent) else {
                return false;
            };
            if !parent_folder.expanded {
                return false;
            }
            current = parent_folder.parent;
        }
        self.folder(id).is_some()
    }

    pub fn visible_folders(&self) -> Vec<&Folder> {
        self.folders
            .iter()
            .filter(|folder| self.is_visible(folder.id))
            .collect()
    }

    fn contains(&self, ancestor: &str, folder: &str) -> bool {
        let mut current = self.folder(folder).map(|folder| folder.id);
        while let Some(id) = current {
            if id == ancestor {
                return true;
            }
            current = self.folder(id).and_then(|folder| folder.parent);
        }
        false
    }

    /// Number of media items in the folder and all of its descendants.
    pub fn count(&self, id: impl AsRef<str>) -> usize {
        let id = id.as_ref();
        self.items
            .iter()
            .filter(|item| self.contains(id, &item.folder))
            .count()
    }

    pub fn set_expanded(&mut self, id: &str, expanded: bool) -> Result<Value, CommandError> {
        if self.folder(id).is_none() {
            return Err(CommandError::unknown_element("folder", id));
        }
        if !self.has_children(id) {
            return Err(
                CommandError::new("not_expandable", "This folder has no sub-folders").target(id),
            );
        }
        let folder = self.folders.iter_mut().find(|folder| folder.id == id);
        let Some(folder) = folder else {
            return Err(CommandError::unknown_element("folder", id));
        };
        let previous = folder.expanded;
        folder.expanded = expanded;
        Ok(
            json!({ "target": format!("folder:{id}"), "field": "expanded",
            "value": expanded, "previous": previous, "changed": previous != expanded }),
        )
    }

    pub fn select_folder(&mut self, id: Option<&str>) -> Result<Value, CommandError> {
        let next = match id {
            None => None,
            Some(id) => Some(
                self.folder(id)
                    .map(|folder| folder.id)
                    .ok_or_else(|| CommandError::unknown_element("folder", id))?,
            ),
        };
        let previous = self.selected_folder;
        self.selected_folder = next;
        Ok(json!({ "target": format!("folder:{}", id.unwrap_or("all")),
            "selected_folder": next, "previous": previous, "changed": previous != next }))
    }

    pub fn set_search(&mut self, query: &str) -> Value {
        let previous = std::mem::replace(&mut self.search, query.to_string());
        json!({ "target": "media.search", "field": "value", "value": self.search,
            "previous": previous, "results": self.results().len() })
    }

    /// Items of the selected bin (recursively) whose name, code or metadata
    /// contains the search text, case-insensitively.
    pub fn results(&self) -> Vec<&MediaItem> {
        let query = self.search.trim().to_lowercase();
        self.items
            .iter()
            .filter(|item| {
                self.selected_folder
                    .is_none_or(|folder| self.contains(&folder, &item.folder))
            })
            .filter(|item| {
                query.is_empty()
                    || [item.name.as_str(), &item.code, &item.meta, &item.id]
                        .iter()
                        .any(|field| field.to_lowercase().contains(&query))
            })
            .collect()
    }

    pub fn bin_label(&self) -> &str {
        self.selected_folder
            .and_then(|id| self.folder(id))
            .map_or("All media", |folder| folder.label.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_roll_up_to_the_parent_bin() {
        let library = Library::default();
        assert_eq!(library.count("footage"), 4);
        assert_eq!(library.count("timelines"), 1);
        assert_eq!(library.count("audio"), 2);
        assert_eq!(library.count(ROOT_FOLDER), 7);
    }

    #[test]
    fn collapsing_a_group_hides_its_children() {
        let mut library = Library::default();
        assert_eq!(library.visible_folders().len(), 4);
        library.set_expanded(ROOT_FOLDER, false).unwrap();
        let visible = library.visible_folders();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].id, ROOT_FOLDER);
        assert!(!library.is_visible("footage"));
        library.set_expanded(ROOT_FOLDER, true).unwrap();
        assert!(library.is_visible("footage"));
    }

    #[test]
    fn leaf_and_unknown_folders_cannot_be_expanded() {
        let mut library = Library::default();
        assert_eq!(
            library.set_expanded("footage", false).unwrap_err().code,
            "not_expandable"
        );
        assert_eq!(
            library.set_expanded("nope", true).unwrap_err().code,
            "unknown_element"
        );
    }

    #[test]
    fn search_filters_the_selected_bin() {
        let mut library = Library::default();
        assert_eq!(library.results().len(), 7);
        library.select_folder(Some("footage")).unwrap();
        assert_eq!(library.results().len(), 4);
        library.set_search("COAST");
        let names: Vec<_> = library
            .results()
            .iter()
            .map(|item| item.name.as_str())
            .collect();
        assert_eq!(names, ["Atlantic coast"]);
        library.set_search("zzz");
        assert!(library.results().is_empty());
        library.select_folder(None).unwrap();
        library.set_search("AU0");
        assert_eq!(library.results().len(), 2);
    }

    #[test]
    fn unknown_folder_selection_is_rejected_without_change() {
        let mut library = Library::default();
        library.select_folder(Some("audio")).unwrap();
        assert!(library.select_folder(Some("missing")).is_err());
        assert_eq!(library.selected_folder, Some(Id::lit("audio")));
    }
}
