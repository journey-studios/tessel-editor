//! Who still relies on a file or folder a *pending* adoption or import created (BL096, BL111).
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
//! # Across processes (BL111)
//!
//! The results being protected live in the memory of one process (a job slot, a
//! `PendingAdoption`, a `PreparedSceneImport`), but the *folder* they write into is shared by
//! every editor, CLI or job that opens the same workspace. So the registry is a **ledger on disk**
//! per workspace ([`ledger`]): `<workspace>/.tessel-claims/`.
//!
//! * A claim is identified by **resource** (canonical path), **identity** (length, device/inode
//!   and SHA-256 of the published file) and **holder** (the owner process plus the result's
//!   sequence number). Every change is made under an exclusive lock on the ledger and published by
//!   an atomic rename, so two processes never interleave a read-modify-write.
//! * A holder is alive while its **owner process holds a lease**: an exclusive `flock` on its own
//!   `live/<token>` file. The kernel drops the lock when the process ends, however it ends
//!   (`kill -9` included), so liveness is decided by the lock, **never by a process ID**: a PID is
//!   only recorded for messages and a recycled PID cannot make a dead holder look alive (or the
//!   reverse).
//! * Whoever opens the ledger first **reaps** the holders whose lease is gone. An entry left with
//!   no holder is an *orphan*: a pending result that no longer exists. It is **kept** until
//!   something decides about it: a new result that reuses the file joins it (and so decides
//!   whether it stays), or [`recover`](super::recover) removes it after the same content proof as
//!   any other cleanup. Nothing here ever removes an accepted file, an entry another live holder
//!   relies on, or a file that is not the published one.
//! * A ledger that cannot be read or written **fails closed**, and *when* it failed decides what
//!   can be promised:
//!   - **before** the adoption changes anything (the ledger cannot be opened or read): it is
//!     refused before copying, `claims_unavailable` with `details.phase = "before_changes"`, and
//!     "nothing was changed" is true;
//!   - **after** it started (a write fails once files are already copied): the file whose claim
//!     could not be written has rollback attempted under the root lease. If proof or removal
//!     fails it can remain unclaimed in `kept`, outside ledger recovery. The adoption is refused with `details.phase = "after_changes"`, which never claims that nothing changed.
//!     Success is only ever returned for claims that are on disk ([`Tracker::create_file`] and
//!     [`Tracker::create_dir`] answer `Ok` only after the write);
//!   - a discard that cannot record leaving ([`Leave::Unverifiable`]) keeps the file, claimed,
//!     and says why; `recover` takes it back after its owner is gone;
//!   - **accepting** (the last step of applying a result) that cannot be written is refused with
//!     `details.phase = "accept"`: see below.
//!
//! The ledger is removed again when the last claim of the last holder is gone, so a workspace
//! with no pending result looks exactly like one that never had any.
//!
//! The registry is keyed by canonical paths, so one folder spelled two ways (`ws/../ws`, a
//! link, a trailing `.`) is still one entry.
//!
//! * **Accepting and merging are written before they are believed.** [`Tracker::accept`] and
//!   [`Tracker::transfer`] answer `Ok` only once the change is on disk, and the owners
//!   ([`Created::accept`](super::Created), `Created::absorb`) only change their own lists after
//!   that answer. A refused acceptance leaves the result pending and claimed, and the flow that
//!   applies it (`import::apply`, `PendingAdoption::apply`) calls it **last** and undoes itself:
//!   the project is as it was. A refused merge leaves both records as they were, so nobody loses
//!   ownership. What a failed write cannot say is whether it reached the disk; every follow-up
//!   is safe either way (a discard only removes a file whose entry it still finds), so the worst
//!   outcome is a file that stays, never one that is deleted under an owner.
//! * **The lease outlives a failed write.** A process lets go of its lease only after the ledger
//!   without it is written. Otherwise a write that failed would leave the disk listing claims
//!   of an owner nobody can see as alive, and the next reader would reap it and `recover` would
//!   remove files a live process owns.
//! * **A state file without its lock is damage.** No lock and no state is "no ledger"; a state
//!   that lists claims with no `ledger.lock` is seen as a fault (after a few looks in a row, so
//!   the one-look races of creating and emptying a ledger are not mistaken for it) and refuses:
//!   `recover` removes nothing, a discard keeps the files, a status says `available: false`. The
//!   path that creates a ledger (a new adoption) takes a fresh lock and keeps reading the state:
//!   it never treats the claims as gone.
//!
//! A destructor still cannot report: a bare [`Created`](super::Created) dropped without
//! `accept` or `remove` is accepted best effort. Nothing that changes a project relies on it.
//!
//! Limits, stated honestly: the lease is an advisory lock of a **local** filesystem (network
//! filesystems may not provide it), a **hostile** writer of the same user is out of scope, and a
//! process that dies after the project accepted a result but before the claims were dropped
//! leaves an orphan that `recover` will verify and may remove (the file was only reachable from
//! a project that died with the process).

mod ledger;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) use ledger::{Ledger, Tracker};

/// One result's identity in the registry: the process that owns it and a number that is never
/// reused within that process. The owner is a random token, not a process ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct HolderId {
    owner: u128,
    seq: u64,
}

impl HolderId {
    /// Pairs a process identity with the sequence of one result; neither component is a PID.
    pub(super) fn new(owner: u128, seq: u64) -> Self {
        Self { owner, seq }
    }

    /// Returns the process token whose live lease protects this result.
    pub(super) fn owner(&self) -> u128 {
        self.owner
    }

    /// Returns the result sequence within its owning process.
    pub(super) fn seq(&self) -> u64 {
        self.seq
    }
}

static NEXT_HOLDER: AtomicU64 = AtomicU64::new(1);

/// Allocates a result identity under this process token without sharing mutable ownership.
pub(super) fn next_holder() -> HolderId {
    HolderId::new(process_owner(), NEXT_HOLDER.fetch_add(1, Ordering::Relaxed))
}

/// The token that names this process in every ledger it touches. Random per process start (the
/// OS entropy source when there is one, mixed with the PID and the clock), so a recycled PID never
/// inherits the claims of the process that had it.
pub(super) fn process_owner() -> u128 {
    static OWNER: OnceLock<u128> = OnceLock::new();
    *OWNER.get_or_init(|| {
        use std::hash::{BuildHasher, Hasher, RandomState};
        use std::io::Read;
        let mut seed = [0u8; 16];
        // Without the OS source (not Unix) the PID, the clock and the hasher's own random keys
        // still make two processes differ.
        let _ = std::fs::File::open("/dev/urandom").and_then(|mut file| file.read_exact(&mut seed));
        let mut mixed = u128::from_le_bytes(seed);
        for round in 0..2u128 {
            let mut hasher = RandomState::new().build_hasher();
            hasher.write_u32(std::process::id());
            hasher.write_u128(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |elapsed| elapsed.as_nanos()),
            );
            mixed ^= u128::from(hasher.finish()) << (64 * round);
        }
        // Never zero, whatever the sources gave.
        mixed | 1
    })
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

/// One resource in the ledger: what it is, which lease root publishes into it, and who relies on
/// it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Entry {
    pub item: Item,
    /// The canonical root whose lease protects a file (`None` for a folder).
    pub root: Option<PathBuf>,
    pub holders: BTreeSet<HolderId>,
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
    /// The ledger cannot vouch for it (unreadable, unwritable): leave it and say why.
    Unverifiable(String),
    /// Only from [`Tracker::abandon`]: nobody holds it any more but it stays claimed, an orphan.
    Abandoned,
}

/// How many entries are tracked at or below `root`: lets a test prove nothing is left behind by
/// results that ended in any way. Orphans (their holder died) still count until recovered.
pub fn tracked_under(root: &Path) -> usize {
    let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let tracker = Ledger::of(root).lock();
    tracker.count_under(&canonical)
}

#[cfg(test)]
mod tests;
