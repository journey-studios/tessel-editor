//! Writing side: what the transport root holds, and the journal that undoes a failed publish.
//!
//! The transport root is canonicalized once by the caller. Below it, nothing may be a symbolic
//! link: writing through one would put a resource outside the authorized root.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use tessel_editor_core::jobs::JobError;
use tessel_editor_core::scene_resources::ResourcePath;

use super::{Failure, ProblemKind};
use crate::fsx::io_error;

/// What is at a destination path now.
#[derive(Debug)]
pub(super) enum Slot {
    Free,
    /// A regular file, to be reused if its content is the expected one.
    File(PathBuf),
}

/// `root/<resource>` with the resource's own separators.
pub(super) fn target(root: &Path, resource: &ResourcePath) -> PathBuf {
    let mut path = root.to_path_buf();
    path.extend(resource.segments());
    path
}

fn conflict(resource: &ResourcePath, why: &str) -> Failure {
    Failure::new(
        ProblemKind::DestinationConflict,
        format!("`{resource}`: {why}"),
    )
}

/// Looks at `resource` under the transport root without changing anything. A symbolic link, a
/// folder where the file belongs or a file where a folder belongs is a conflict.
pub(super) fn inspect(root: &Path, resource: &ResourcePath) -> Result<Slot, Failure> {
    let mut current = root.to_path_buf();
    let segments: Vec<&str> = resource.segments().collect();
    for (index, segment) in segments.iter().enumerate() {
        current.push(*segment);
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Slot::Free),
            Err(error) => {
                return Err(Failure::new(
                    ProblemKind::Unreadable,
                    format!("`{resource}` in the transport root: {error}"),
                ));
            }
        };
        if metadata.file_type().is_symlink() {
            return Err(conflict(
                resource,
                "a symbolic link is in the way in the transport root",
            ));
        }
        let is_last = index + 1 == segments.len();
        if is_last && metadata.is_file() {
            return Ok(Slot::File(current));
        }
        if is_last {
            return Err(conflict(
                resource,
                "something that is not a regular file is in the way in the transport root",
            ));
        }
        if !metadata.is_dir() {
            return Err(conflict(
                resource,
                "a file is where a folder is needed in the transport root",
            ));
        }
    }
    // `ResourcePath` has at least one segment, so the loop always returns.
    Ok(Slot::Free)
}

/// Everything this run created under the transport root, so a failure can remove exactly that.
#[derive(Debug)]
pub(super) struct Journal {
    /// Canonical transport root: every published file must still resolve below it.
    root: PathBuf,
    files: Vec<PathBuf>,
    dirs: Vec<PathBuf>,
}

impl Journal {
    pub(super) fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            files: Vec::new(),
            dirs: Vec::new(),
        }
    }

    /// Creates the folders of `resource` that do not exist yet, one at a time (never
    /// `create_dir_all`, which would follow a link), and records each one it made.
    pub(super) fn create_parents(
        &mut self,
        root: &Path,
        resource: &ResourcePath,
    ) -> Result<(), JobError> {
        let segments: Vec<&str> = resource.segments().collect();
        let mut current = root.to_path_buf();
        for segment in &segments[..segments.len() - 1] {
            current.push(*segment);
            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.is_dir() => {}
                Ok(_) => {
                    return Err(JobError::new(
                        "destination_conflict",
                        format!("`{resource}`: a folder of the path is not a plain folder"),
                    ));
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    fs::create_dir(&current).map_err(|error| io_error(&current, &error))?;
                    self.dirs.push(current.clone());
                }
                Err(error) => return Err(io_error(&current, &error)),
            }
        }
        Ok(())
    }

    /// Moves a staged file to `destination` without replacing anything. A hard link is created
    /// first because linking fails if the destination exists, which closes the window between
    /// "nothing is there" and "put it there". Other failures are returned: check-then-rename
    /// would permit replacing a destination created by another writer in between.
    ///
    /// The parent is resolved before and the new link after: a parent that was swapped for a
    /// link to somewhere else between the folder pass and now (BL079; the lease keeps other
    /// Tessel operations out but not a hostile writer) is caught here and the stray link is
    /// removed. This narrows that window, it does not close it: see `lease.rs`.
    pub(super) fn publish(&mut self, staged: &Path, destination: &Path) -> Result<(), JobError> {
        if let Some(parent) = destination.parent() {
            let real = fs::canonicalize(parent).map_err(|error| io_error(parent, &error))?;
            if !real.starts_with(&self.root) {
                return Err(escaped(destination));
            }
        }
        match fs::hard_link(staged, destination) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                return Err(exists(destination));
            }
            Err(error) => return Err(io_error(destination, &error)),
        }
        // Recorded before the check so a rollback also removes a link that landed outside.
        self.files.push(destination.to_path_buf());
        match fs::canonicalize(destination) {
            Ok(real) if real.starts_with(&self.root) => Ok(()),
            _ => Err(escaped(destination)),
        }
    }

    /// Removes what this run created, files first and folders deepest first. Returns what could
    /// not be removed (empty on a clean rollback).
    pub(super) fn rollback(&mut self) -> Vec<String> {
        let mut leftovers = Vec::new();
        for file in self.files.drain(..).rev() {
            if let Err(error) = fs::remove_file(&file) {
                leftovers.push(format!("`{}`: {error}", file.display()));
            }
        }
        for dir in self.dirs.drain(..).rev() {
            if let Err(error) = fs::remove_dir(&dir) {
                leftovers.push(format!("`{}`: {error}", dir.display()));
            }
        }
        leftovers
    }
}

fn escaped(destination: &Path) -> JobError {
    JobError::new(
        "destination_escaped",
        format!(
            "`{}` would resolve outside the transport root; the file was not kept",
            destination.display()
        ),
    )
}

fn exists(destination: &Path) -> JobError {
    JobError::new(
        "destination_conflict",
        format!(
            "`{}` appeared while publishing; nothing was overwritten",
            destination.display()
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fsx::Staging;

    #[test]
    fn failed_hard_link_never_falls_back_to_a_rename() {
        let sandbox = Staging::create(&std::env::temp_dir(), "publish-no-replace").unwrap();
        let source = sandbox.path().join("unexpected-directory");
        let destination = sandbox.path().join("output");
        fs::create_dir(&source).unwrap();
        let mut journal = Journal::new(&fs::canonicalize(sandbox.path()).unwrap());
        // Linking a directory fails. A check-then-rename fallback would still move it,
        // and has no atomic no-replace guarantee if another writer creates the leaf.
        assert!(journal.publish(&source, &destination).is_err());
        assert!(source.is_dir());
        assert!(!destination.exists());
        assert!(journal.files.is_empty());
    }
}
