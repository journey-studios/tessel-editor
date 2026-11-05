//! Who still relies on a file or folder a *pending* adoption or import created (BL096).
//!
//! `adopt` reuses a file that is already in the destination with the expected hash. When that
//! file was copied by another result that nobody has accepted yet, the two results share it: the
//! first one discarded must not delete what the second one (accepted, or still waiting) relies
//! on. A discard that only *notices afterwards* is too late, so ownership is decided up front:
//!
//! * Whatever a pending result **creates** is entered here with the result as its holder.
//! * A result that **reuses** an entry (or puts a new file in a folder that has one) joins it as
//!   another holder, while it still holds the root lease of the publication, so no discard can
//!   interleave between "I will reuse it" and "I rely on it".
//! * **Accepting** a result ([`Tracker::accept`]) deletes its entries: from then on the files
//!   belong to the project and nobody removes them, whichever holder leaves later.
//! * **Leaving** without accepting ([`Tracker::leave`]) removes the holder; only the last one
//!   to leave is told it may delete, and only an entry that a pending result created exists at
//!   all. What was in the workspace before, or was accepted, has no entry and is never removed.
//!
//! WHY a process-wide registry and not a ledger on disk: the results being protected live in the
//! memory of one process (a job slot, a `PendingAdoption`, a `PreparedSceneImport`), so their
//! owner is the process. A second *process* that reuses the same file is not seen here (it would
//! need a claim file per holder kept alive by a lock, and a rule for the claims of a crashed
//! process). That is a stated limit of F040, not a guarantee: see the feature document.
//!
//! The registry is keyed by canonical paths, so one folder spelled two ways (`ws/../ws`, a
//! link, a trailing `.`) is still one entry.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard};

/// One result's identity in the registry. Never reused within a process.
pub(super) type HolderId = u64;

static NEXT_HOLDER: AtomicU64 = AtomicU64::new(1);

pub(super) fn next_holder() -> HolderId {
    NEXT_HOLDER.fetch_add(1, Ordering::Relaxed)
}

/// What a file was when it was published: the cheap identity (length and, on Unix, device and
/// inode) plus the SHA-256 the bundle declares and `materialize` verified before publishing.
///
/// The hash is what makes the stamp decisive: a file rewritten **in place** keeps its inode and,
/// when the writer writes as many bytes as before, its length too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Stamp {
    pub len: u64,
    #[cfg(unix)]
    pub device: u64,
    #[cfg(unix)]
    pub inode: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Item {
    /// `None` when the file could not be inspected right after publishing: it is tracked (so it
    /// is not mistaken for an accepted one) but cannot be proven to be the published file.
    File(Option<Stamp>),
    Dir,
}

#[derive(Debug)]
struct Entry {
    item: Item,
    holders: BTreeSet<HolderId>,
}

/// What [`Tracker::leave`] tells the one leaving.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Leave {
    /// Not a pending result's creation (it was there before, or someone accepted it): leave it.
    Untracked,
    /// Another result still relies on it: leave it.
    Shared,
    /// The last holder left: the creator side may remove it, after checking what it is.
    Last(Item),
}

static REGISTRY: LazyLock<Mutex<HashMap<PathBuf, Entry>>> = LazyLock::new(Mutex::default);

/// The registry, locked. Hold it only across short, non-blocking work (no hashing, no waiting
/// for a lease): the only file operation done under it is creating or removing one folder, which
/// is what keeps "the folder exists" and "its entry exists" true together.
pub(super) struct Tracker(MutexGuard<'static, HashMap<PathBuf, Entry>>);

pub(super) fn lock() -> Tracker {
    // A holder that panicked left consistent data (every change is one insert/remove).
    Tracker(
        REGISTRY
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    )
}

impl Tracker {
    /// `holder` created the file at `key`.
    pub(super) fn create_file(&mut self, holder: HolderId, key: PathBuf, stamp: Option<Stamp>) {
        self.insert(holder, key, Item::File(stamp));
    }

    /// `holder` created the folder at `key`.
    pub(super) fn create_dir(&mut self, holder: HolderId, key: PathBuf) {
        self.insert(holder, key, Item::Dir);
    }

    // A stale entry at a path that was just created cannot be a live holder's: whoever listed it
    // removed or lost the file first. The new entry replaces it.
    fn insert(&mut self, holder: HolderId, key: PathBuf, item: Item) {
        self.0.insert(
            key,
            Entry {
                item,
                holders: BTreeSet::from([holder]),
            },
        );
    }

    /// `holder` relies on the entry at `key`, if a pending result created one. Answers whether
    /// there was one.
    pub(super) fn join(&mut self, holder: HolderId, key: &Path) -> bool {
        let Some(entry) = self.0.get_mut(key) else {
            return false;
        };
        entry.holders.insert(holder);
        true
    }

    /// `holder` gives up the entry at `key` without accepting.
    pub(super) fn leave(&mut self, holder: HolderId, key: &Path) -> Leave {
        let Some(entry) = self.0.get_mut(key) else {
            return Leave::Untracked;
        };
        entry.holders.remove(&holder);
        if !entry.holders.is_empty() {
            return Leave::Shared;
        }
        match self.0.remove(key) {
            Some(entry) => Leave::Last(entry.item),
            None => Leave::Untracked,
        }
    }

    /// The result that listed `keys` was accepted: the project owns them now, for every holder.
    pub(super) fn accept<'a>(&mut self, keys: impl Iterator<Item = &'a Path>) {
        for key in keys {
            self.0.remove(key);
        }
    }

    /// Merges the holder `from` into `to` for the entries at `keys` (two results of one
    /// operation becoming one).
    pub(super) fn transfer<'a>(
        &mut self,
        from: HolderId,
        to: HolderId,
        keys: impl Iterator<Item = &'a Path>,
    ) {
        for key in keys {
            if let Some(entry) = self.0.get_mut(key)
                && entry.holders.remove(&from)
            {
                entry.holders.insert(to);
            }
        }
    }
}

/// How many entries are tracked at or below `root`: lets a test prove nothing is left behind by
/// results that ended in any way.
pub fn tracked_under(root: &Path) -> usize {
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    lock().0.keys().filter(|key| key.starts_with(&root)).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(name: &str) -> PathBuf {
        PathBuf::from(format!("/te-claims-unit/{name}"))
    }

    #[test]
    fn only_the_last_holder_to_leave_may_remove() {
        let (one, two, path) = (next_holder(), next_holder(), key("last-leaves"));
        let mut tracker = lock();
        tracker.create_file(one, path.clone(), None);
        assert!(tracker.join(two, &path));
        assert_eq!(tracker.leave(one, &path), Leave::Shared);
        assert_eq!(tracker.leave(two, &path), Leave::Last(Item::File(None)));
        assert_eq!(tracker.leave(two, &path), Leave::Untracked, "gone for good");
    }

    #[test]
    fn accepting_makes_the_entry_nobodys_to_remove() {
        let (one, two, path) = (next_holder(), next_holder(), key("accepted"));
        let mut tracker = lock();
        tracker.create_dir(one, path.clone());
        tracker.join(two, &path);
        tracker.accept([path.as_path()].into_iter());
        assert_eq!(tracker.leave(one, &path), Leave::Untracked);
        assert_eq!(tracker.leave(two, &path), Leave::Untracked);
    }

    #[test]
    fn joining_something_nobody_created_is_a_no_op() {
        let path = key("never-created");
        let mut tracker = lock();
        assert!(!tracker.join(next_holder(), &path));
        assert_eq!(tracker.leave(next_holder(), &path), Leave::Untracked);
    }

    #[test]
    fn a_transferred_holder_keeps_the_entry_alive_under_its_new_name() {
        let (from, to, other, path) = (
            next_holder(),
            next_holder(),
            next_holder(),
            key("transferred"),
        );
        let mut tracker = lock();
        tracker.create_file(from, path.clone(), None);
        tracker.join(other, &path);
        tracker.transfer(from, to, [path.as_path()].into_iter());
        assert_eq!(tracker.leave(other, &path), Leave::Shared);
        assert_eq!(tracker.leave(to, &path), Leave::Last(Item::File(None)));
    }
}
