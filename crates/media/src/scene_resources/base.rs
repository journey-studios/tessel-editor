//! Where the files a bundle names by relative path live: an *explicit* base, never the working
//! directory.
//!
//! A bundle's `relativePath` is relative to something, and "something" has to be registered, not
//! guessed. Two kinds exist:
//!
//! * **Managed** (the persistent, transportable one): `<workspace>/scenes/<sceneId>/`. It is a
//!   pure function of the project's workspace (the folder of its document) and the scene ID, so
//!   nothing about it is stored, it moves with the project folder, and a restored package puts
//!   the very same layout under its own root. This is the base a project *keeps*.
//! * **Explicit**: an absolute folder the caller names for one operation (the folder a bundle was
//!   authored in). It is session data, not part of the project, and is never packaged as such:
//!   `scene.resources.adopt` copies from it into the managed base.
//!
//! WHY derive instead of store: the project document belongs to the core, which is not changed by
//! this slice. A stored base would need a new document key; the convention needs none and cannot
//! go stale when the folder is moved or zipped. Persisting a *reference* base (resources left in
//! place) is a separate, proposed core change (`docs/features/scene-resource-integration.md`).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tessel_editor_core::jobs::JobError;
use tessel_editor_core::scene_project::check_scene_id;

/// Folder of the workspace (and of a package) that holds one subfolder per scene.
pub const MANAGED_DIR: &str = "scenes";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseKind {
    Managed,
    Explicit,
    /// No folder at all: what a scene that names no files loads against (BL101).
    Unbound,
}

impl BaseKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Managed => "managed",
            Self::Explicit => "explicit",
            Self::Unbound => "unbound",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceBase {
    root: PathBuf,
    kind: BaseKind,
}

impl ResourceBase {
    /// A folder the caller names. It must be absolute: a relative one would silently mean
    /// "relative to wherever the process happens to run".
    pub fn explicit(root: impl Into<PathBuf>) -> Result<Self, JobError> {
        let root = root.into();
        if !root.is_absolute() {
            return Err(not_absolute("resource base", &root));
        }
        Ok(Self {
            root,
            kind: BaseKind::Explicit,
        })
    }

    /// `<workspace>/scenes/<scene_id>`, which may not exist yet (adoption creates it).
    pub fn managed(workspace: &Path, scene_id: &str) -> Result<Self, JobError> {
        if !workspace.is_absolute() {
            return Err(not_absolute("workspace", workspace));
        }
        let folder = folder_name(scene_id)?;
        Ok(Self {
            root: workspace.join(MANAGED_DIR).join(folder),
            kind: BaseKind::Managed,
        })
    }

    /// A base that is nowhere. It is enough for a scene whose manifest names no files (nothing is
    /// opened, so nothing can escape), and for nothing else: a scene that names a file still
    /// fails the preflight with `root_unavailable`, because an empty path is never a folder.
    pub fn unbound() -> Self {
        Self {
            root: PathBuf::new(),
            kind: BaseKind::Unbound,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn kind(&self) -> BaseKind {
        self.kind
    }

    pub fn exists(&self) -> bool {
        self.root.is_dir()
    }

    pub fn to_json(&self) -> Value {
        json!({ "kind": self.kind.as_str(), "root": self.root })
    }
}

fn not_absolute(what: &str, path: &Path) -> JobError {
    JobError::new(
        "base_not_absolute",
        format!(
            "the {what} `{}` is not an absolute path; relative bases are never resolved against the working directory",
            path.display()
        ),
    )
}

/// The folder name of a scene: its ID, when that is also a safe, portable folder name. Scene IDs
/// share the alphabet of `Id` (`check_scene_id`), which still allows `.` and `..`, a leading dot
/// (a hidden folder, and the `.tessel-staging-` prefix is reserved) and, on case-insensitive
/// volumes, two IDs that differ only by case.
pub fn folder_name(scene_id: &str) -> Result<&str, JobError> {
    check_scene_id(scene_id)?;
    if scene_id.starts_with('.') {
        return Err(JobError::new(
            "scene_id_not_portable",
            format!(
                "the scene ID `{scene_id}` starts with a dot, so it cannot name a resource folder"
            ),
        ));
    }
    Ok(scene_id)
}

/// `scenes/<sceneId>` inside a project folder or a package, with `/` as separator.
pub fn entry_prefix(scene_id: &str) -> Result<String, JobError> {
    Ok(format!("{MANAGED_DIR}/{}", folder_name(scene_id)?))
}

/// Pairs of IDs that name the same folder on a case-insensitive volume (one file system folder
/// for two scenes would mix their resources).
pub fn folding_collisions<'a>(ids: impl IntoIterator<Item = &'a str>) -> Vec<(String, String)> {
    let mut seen: BTreeMap<String, &str> = BTreeMap::new();
    let mut found = Vec::new();
    for id in ids {
        let folded = id.to_lowercase();
        match seen.get(&folded) {
            Some(first) if *first != id => found.push(((*first).to_string(), id.to_string())),
            Some(_) => {}
            None => {
                seen.insert(folded, id);
            }
        }
    }
    found
}

/// Refuses to write the managed base of `scene_id` when a different folder that folds to the
/// same name already exists under `<workspace>/scenes/`, or when `scenes/` or the scene folder
/// is a link (writing through one would leave the workspace).
pub(crate) fn check_managed_destination(workspace: &Path, scene_id: &str) -> Result<(), JobError> {
    let folder = folder_name(scene_id)?;
    let scenes = workspace.join(MANAGED_DIR);
    for path in [scenes.clone(), scenes.join(folder)] {
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(JobError::new(
                    "destination_conflict",
                    format!(
                        "`{}` is a symbolic link; scene resources are never written through one",
                        path.display()
                    ),
                ));
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(JobError::new(
                    "destination_conflict",
                    format!("`{}` is not a folder", path.display()),
                ));
            }
            _ => {}
        }
    }
    let Ok(entries) = fs::read_dir(&scenes) else {
        return Ok(());
    };
    let wanted = folder.to_lowercase();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name != folder && name.to_lowercase() == wanted {
            return Err(JobError::new(
                "scene_id_collision",
                format!(
                    "`{scenes}/{name}` already holds the resources of another scene that differs from `{folder}` only by letter case; on a case-insensitive volume they would be one folder",
                    scenes = scenes.display()
                ),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relative_base_is_refused_instead_of_meaning_the_working_directory() {
        assert_eq!(
            ResourceBase::explicit("assets").unwrap_err().code,
            "base_not_absolute"
        );
        assert_eq!(
            ResourceBase::managed(Path::new("ws"), "a")
                .unwrap_err()
                .code,
            "base_not_absolute"
        );
    }

    #[test]
    fn the_managed_base_is_derived_from_the_workspace_and_the_scene_id() {
        let base = ResourceBase::managed(Path::new("/p/ws"), "street").unwrap();
        assert_eq!(base.root(), Path::new("/p/ws/scenes/street"));
        assert_eq!(base.kind(), BaseKind::Managed);
        assert_eq!(entry_prefix("street").unwrap(), "scenes/street");
    }

    #[test]
    fn ids_that_cannot_be_a_folder_are_refused() {
        for id in [".", "..", ".hidden", ".tessel-staging-x", "a/b", ""] {
            assert!(folder_name(id).is_err(), "{id:?}");
        }
        assert_eq!(folder_name("a.b-c_d").unwrap(), "a.b-c_d");
    }

    #[test]
    fn ids_that_differ_only_by_case_are_reported() {
        assert_eq!(
            folding_collisions(["Street", "market", "street"]),
            [("Street".to_string(), "street".to_string())]
        );
        assert!(folding_collisions(["a", "b"]).is_empty());
    }
}
