//! Reading side: finds a resource file under the source root without ever leaving it.
//!
//! WHY a component walk and not `root.join(path)` plus a prefix test: joining keeps symbolic links
//! as they are, so `root/assets` can be a link to `/etc` and a purely textual check still passes.
//! Each component is therefore inspected with `symlink_metadata`, the final path is canonicalized
//! and compared with the canonical root by components (not by string prefix), and the handle that
//! is finally opened is checked to be the very file that was verified.

use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

use tessel_editor_core::jobs::JobError;
use tessel_editor_core::scene_resources::ResourcePath;

use super::{Failure, Options, ProblemKind, SymlinkPolicy};

/// A directory that was canonicalized once: every later check is against that fixed path.
#[derive(Debug)]
pub(super) struct Root {
    canonical: PathBuf,
}

/// A verified, open regular file.
#[derive(Debug)]
pub(super) struct SourceFile {
    pub file: File,
    /// Canonical path, inside the root.
    pub resolved: PathBuf,
    pub size: u64,
}

impl Root {
    pub(super) fn open(path: &Path, label: &str) -> Result<Self, JobError> {
        let canonical = fs::canonicalize(path).map_err(|error| {
            JobError::new(
                "root_unavailable",
                format!("the {label} root `{}`: {error}", path.display()),
            )
        })?;
        if !canonical.is_dir() {
            return Err(JobError::new(
                "root_not_directory",
                format!("the {label} root `{}` is not a directory", path.display()),
            ));
        }
        Ok(Self { canonical })
    }

    pub(super) fn path(&self) -> &Path {
        &self.canonical
    }

    /// Opens `resource` for reading, or says why it may not be read.
    pub(super) fn open_file(
        &self,
        resource: &ResourcePath,
        options: &Options,
    ) -> Result<SourceFile, Failure> {
        let mut current = self.canonical.clone();
        let segments: Vec<&str> = resource.segments().collect();
        for (index, segment) in segments.iter().enumerate() {
            current.push(*segment);
            let metadata =
                fs::symlink_metadata(&current).map_err(|error| lookup_failure(resource, &error))?;
            if metadata.file_type().is_symlink() {
                if options.symlinks == SymlinkPolicy::Reject {
                    return Err(Failure::new(
                        ProblemKind::SymlinkRejected,
                        format!("`{resource}` goes through a symbolic link"),
                    ));
                }
                continue;
            }
            let is_last = index + 1 == segments.len();
            if is_last && !metadata.is_file() {
                return Err(Failure::new(
                    ProblemKind::NotRegularFile,
                    format!("`{resource}` is not a regular file"),
                ));
            }
            if !is_last && !metadata.is_dir() {
                return Err(Failure::new(
                    ProblemKind::NotRegularFile,
                    format!("a folder of `{resource}` is not a folder"),
                ));
            }
        }

        let resolved =
            fs::canonicalize(&current).map_err(|error| lookup_failure(resource, &error))?;
        if !resolved.starts_with(&self.canonical) {
            return Err(Failure::new(
                ProblemKind::EscapesRoot,
                format!("`{resource}` resolves outside the source root"),
            ));
        }
        let verified = fs::metadata(&resolved).map_err(|error| lookup_failure(resource, &error))?;
        if !verified.is_file() {
            return Err(Failure::new(
                ProblemKind::NotRegularFile,
                format!("`{resource}` is not a regular file"),
            ));
        }
        if verified.len() > options.max_resource_bytes {
            return Err(Failure::new(
                ProblemKind::TooLarge,
                format!(
                    "`{resource}` has {} bytes; the limit is {}",
                    verified.len(),
                    options.max_resource_bytes
                ),
            ));
        }
        let file = File::open(&resolved).map_err(|error| lookup_failure(resource, &error))?;
        let opened = file
            .metadata()
            .map_err(|error| lookup_failure(resource, &error))?;
        ensure_same_file(resource, &verified, &opened)?;
        Ok(SourceFile {
            file,
            resolved,
            size: opened.len(),
        })
    }
}

fn lookup_failure(resource: &ResourcePath, error: &io::Error) -> Failure {
    match error.kind() {
        io::ErrorKind::NotFound => Failure::new(
            ProblemKind::Missing,
            format!("`{resource}` does not exist under the source root"),
        ),
        io::ErrorKind::NotADirectory => Failure::new(
            ProblemKind::NotRegularFile,
            format!("a folder of `{resource}` is a file"),
        ),
        _ => Failure::new(ProblemKind::Unreadable, format!("`{resource}`: {error}")),
    }
}

/// The path may have been swapped between `canonicalize` and `open`; the opened handle has to be
/// the file whose metadata was checked. Same device and inode on Unix; elsewhere the check is
/// skipped (limit: another process with write access to the source root can still race it).
#[cfg(unix)]
fn ensure_same_file(
    resource: &ResourcePath,
    verified: &fs::Metadata,
    opened: &fs::Metadata,
) -> Result<(), Failure> {
    use std::os::unix::fs::MetadataExt;
    if verified.dev() == opened.dev() && verified.ino() == opened.ino() {
        return Ok(());
    }
    Err(Failure::new(
        ProblemKind::Changed,
        format!("`{resource}` was replaced while it was being opened"),
    ))
}

#[cfg(not(unix))]
fn ensure_same_file(
    _resource: &ResourcePath,
    _verified: &fs::Metadata,
    _opened: &fs::Metadata,
) -> Result<(), Failure> {
    Ok(())
}
