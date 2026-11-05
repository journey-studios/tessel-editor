//! Exclusive lease on a transport root (BL079): one writer at a time among cooperating Tessel
//! operations (jobs of one host, two editor processes, the CLI next to the window).
//!
//! WHY an advisory lock and not directory-anchored (`openat`) writes: the standard library has
//! no directory handles and the workspace has no `libc`/`rustix` dependency, so a plain path
//! cannot be defended against a *hostile* writer swapping a parent directory for a link between
//! two path operations. What a lease does guarantee is that two Tessel operations never publish
//! into the same root at once (adoption vs. package vs. restore vs. a second window), which is
//! the race the editor can actually cause itself. Hostile same-user writers are out of scope and
//! documented as such (`docs/features/scene-resource-integration.md`); the per-file containment
//! check in `destination.rs` narrows, but does not close, that window.
//!
//! The lock file lives *inside* the root (the only place that is certainly writable for the
//! caller and certainly not shared with another root) and is removed on release so a root that
//! an operation refused or rolled back is byte-for-byte as it was. The name uses the reserved
//! `.tessel-staging-` prefix, which `ResourcePath` rejects, so no resource can collide with it.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde_json::json;
use tessel_editor_core::jobs::JobError;

use super::source::Root;
use crate::fsx::io_error;

pub const LEASE_FILE: &str = ".tessel-staging-lock";

/// How many times a lease re-opens the lock file when the one it locked was unlinked by a
/// releasing holder in between (see [`RootLease::acquire`]).
const ATTEMPTS: usize = 5;

/// Held for as long as the value lives. Dropping it releases the root.
#[derive(Debug)]
pub struct RootLease {
    root: PathBuf,
    lock_path: PathBuf,
    file: Option<File>,
}

impl RootLease {
    /// Takes the lease or fails at once with `root_busy` (it never waits: a job that waits on
    /// another job's root would hide a stuck operation behind a progress bar).
    pub fn acquire(root: &Path) -> Result<Self, JobError> {
        let canonical = Root::open(root, "transport")?.path().to_path_buf();
        let lock_path = canonical.join(LEASE_FILE);
        for _ in 0..ATTEMPTS {
            let mut file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&lock_path)
                .map_err(|error| io_error(&lock_path, &error))?;
            match file.try_lock() {
                Ok(()) => {}
                Err(TryLockError::WouldBlock) => return Err(busy(&canonical, &mut file)),
                Err(TryLockError::Error(error)) => return Err(io_error(&lock_path, &error)),
            }
            // A holder that released just before our lock removed the file we hold now: the
            // lock is real but on an inode nobody else will ever find. Retry on the live path.
            if !is_still_the_lock_file(&lock_path, &file) {
                continue;
            }
            let _ = file.set_len(0);
            let _ = write!(file, "{}", std::process::id());
            return Ok(Self {
                root: canonical,
                lock_path,
                file: Some(file),
            });
        }
        Err(JobError::new(
            "root_busy",
            format!(
                "`{}` is being leased and released too quickly to be taken",
                canonical.display()
            ),
        ))
    }

    /// The canonical root this lease protects.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl Drop for RootLease {
    fn drop(&mut self) {
        // Unlink first, unlock after: a process that opened the old file in between sees the
        // mismatch in `is_still_the_lock_file` and retries.
        let _ = fs::remove_file(&self.lock_path);
        drop(self.file.take());
    }
}

fn busy(root: &Path, file: &mut File) -> JobError {
    let mut holder = String::new();
    let _ = file.read_to_string(&mut holder);
    let holder = holder.trim().parse::<u32>().ok();
    JobError::new(
        "root_busy",
        format!(
            "`{}` is in use by another Tessel operation (process {}); nothing was changed",
            root.display(),
            holder.map_or_else(|| "unknown".to_string(), |pid| pid.to_string())
        ),
    )
    .with_details(json!({ "root": root, "holder": holder }))
}

#[cfg(unix)]
fn is_still_the_lock_file(path: &Path, file: &File) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (fs::metadata(path), file.metadata()) {
        (Ok(on_disk), Ok(open)) => on_disk.dev() == open.dev() && on_disk.ino() == open.ino(),
        _ => false,
    }
}

/// Without device and inode numbers the check cannot be made; the lease then protects only
/// against operations that never unlink (limit documented for non-Unix platforms).
#[cfg(not(unix))]
fn is_still_the_lock_file(path: &Path, _file: &File) -> bool {
    path.exists()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fsx::Staging;

    fn sandbox(name: &str) -> Staging {
        Staging::create(&std::env::temp_dir(), name).unwrap()
    }

    #[test]
    fn a_second_lease_on_the_same_root_is_refused_until_the_first_is_dropped() {
        let dir = sandbox("lease-exclusive");
        let first = RootLease::acquire(dir.path()).unwrap();
        let error = RootLease::acquire(dir.path()).unwrap_err();
        assert_eq!(error.code, "root_busy");
        assert_eq!(
            error.details.as_ref().unwrap()["holder"],
            std::process::id(),
            "the refusal names who holds the root"
        );
        drop(first);
        drop(RootLease::acquire(dir.path()).unwrap());
    }

    #[test]
    fn releasing_leaves_the_root_exactly_as_it_was() {
        let dir = sandbox("lease-clean");
        fs::write(dir.path().join("keep"), b"x").unwrap();
        drop(RootLease::acquire(dir.path()).unwrap());
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["keep"]);
    }

    #[test]
    fn leases_of_different_roots_do_not_interfere() {
        let (one, two) = (sandbox("lease-a"), sandbox("lease-b"));
        let first = RootLease::acquire(one.path()).unwrap();
        let second = RootLease::acquire(two.path()).unwrap();
        assert_ne!(first.root(), second.root());
    }

    #[test]
    fn a_missing_root_is_the_usual_root_error() {
        let dir = sandbox("lease-missing");
        let error = RootLease::acquire(&dir.path().join("nowhere")).unwrap_err();
        assert_eq!(error.code, "root_unavailable");
    }
}
