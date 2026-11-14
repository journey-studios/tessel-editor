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
//!
//! On Windows the lock is mandatory (nobody else can read the file while it is held) and an
//! unlinked file can be replaced while a waiter still holds it open: `crate::lock_file` says
//! how the holder is named and how a lock on a replaced file is recognised there (BL168.6.1).

use std::fs::{self, File, OpenOptions, TryLockError};
#[cfg(not(windows))]
use std::io::Read;
use std::io::Write;
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
                Err(TryLockError::WouldBlock) => {
                    return Err(busy(&canonical, holder(&lock_path, &mut file)));
                }
                Err(TryLockError::Error(error)) => return Err(io_error(&lock_path, &error)),
            }
            // A holder that released just before our lock removed the file we hold now: the
            // lock is real but on an inode nobody else will ever find. Retry on the live path.
            if !is_still_the_lock_file(&lock_path, &file) {
                continue;
            }
            let _ = file.set_len(0);
            let _ = write!(file, "{}", std::process::id());
            // The lock makes the file itself unreadable to everyone else on Windows.
            #[cfg(windows)]
            crate::lock_file::write_holder(&lock_path, std::process::id());
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

/// The process ID the holder wrote into the lock file `file` (at `_lock_path`), if it can be read.
#[cfg(not(windows))]
fn holder(_lock_path: &Path, file: &mut File) -> Option<u32> {
    let mut holder = String::new();
    let _ = file.read_to_string(&mut holder);
    holder.trim().parse::<u32>().ok()
}

/// Windows locks are mandatory: the locked file cannot be read from this handle (`os error 33`),
/// so the holder is read from the stream it wrote for that (see `crate::lock_file`).
#[cfg(windows)]
fn holder(lock_path: &Path, _file: &mut File) -> Option<u32> {
    crate::lock_file::read_holder(lock_path)
}

fn busy(root: &Path, holder: Option<u32>) -> JobError {
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

/// No device and inode numbers on Windows: the holder stamps the file it locked and looks for
/// the stamp at the path (`crate::lock_file`). A file that kept the name exists in both cases.
#[cfg(windows)]
fn is_still_the_lock_file(path: &Path, file: &File) -> bool {
    crate::lock_file::is_still_at(path, file)
}

/// Without device and inode numbers the check cannot be made; the lease then protects only
/// against operations that never unlink (limit documented for other platforms).
#[cfg(not(any(unix, windows)))]
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
