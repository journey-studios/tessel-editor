//! The claims ledger of one workspace, on disk (BL111): `<workspace>/.tessel-claims/`.
//!
//! ```text
//! .tessel-claims/
//!   ledger.lock     exclusive flock held for a Tracker scope, possibly several writes
//!   claims.json     owners and entries (version 1), replaced by an atomic rename
//!   live/<token>    one per owner process, flocked for as long as that process lives
//! ```
//!
//! WHY two kinds of lock. `ledger.lock` serialises *changes*. A Tracker may keep it across a
//! batch of ledger rewrites, so a healthy recovery can hold it longer than the tick budget.
//! `live/<token>` is the **lease** that says an owner is still
//! there: the kernel releases it when the process ends, however it ends, so a second process that
//! can take it knows the owner is gone. Neither depends on a process ID (see `mod.rs`).
//!
//! Ordering rules that keep it consistent without a daemon:
//!
//! * An owner's `live/<token>` is created, locked and written into `owners` **in the same critical
//!   section** that first names it as a holder, and is removed in the one that drops its last
//!   holder. So under `ledger.lock` "listed owner without a lockable lease" always means dead.
//! * `ledger.lock` may be unlinked by whoever empties the ledger. A waiter that locked the
//!   unlinked file notices (path and open file are not the same inode) and starts again.
//! * Lock order across the module: root lease, then `ledger.lock`. Nothing waits for a root
//!   lease while holding `ledger.lock`.

use std::collections::{BTreeMap, BTreeSet, HashMap, btree_map};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

use super::{Entry, HolderId, Item, Leave, Stamp, process_owner};

/// Name of the folder inside the workspace.
pub const DIR: &str = ".tessel-claims";
const LOCK: &str = "ledger.lock";
const STATE: &str = "claims.json";
const STATE_TMP: &str = "claims.json.tmp";
const LIVE: &str = "live";
const VERSION: u64 = 1;
/// How many times a lock is retaken when the lock file it got was unlinked meanwhile (or the
/// open raced with that unlink).
const ATTEMPTS: usize = 64;
/// Consecutive looks that find the state without its lock before that is called damage, and the
/// pause between looks.
const DANGLING_SEEN: usize = 5;
const DANGLING_PAUSE: std::time::Duration = std::time::Duration::from_millis(2);
/// How long a caller that may block (a job, the CLI) waits for `ledger.lock` before calling the
/// ledger unavailable (BL112.3), and how often it looks while waiting. Short writes usually
/// release it quickly, but a healthy recovery can hold it longer while repeatedly rewriting
/// the ledger for its orphans; the limit is not restricted to stopped or stuck holders.
///
/// **Not for the window's tick**: 1.5 s is far above its 100 ms ceiling. The tick waits only
/// [`TICK_WAIT`] (see [`Ledger::try_lock`]).
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_millis(1500);
/// How long the window's tick waits for `ledger.lock` (BL112.3, BL112.5): a fixed budget well
/// below the UI's 100 ms ceiling, sized to ride out brief contention. A healthy recovery
/// may keep the lock across several writes and exhaust this budget. `flock` conflicts between descriptors **of the same process** too (every
/// [`Tracker`] opens its own), so a job of this process (a discard, an adoption, a recovery)
/// holding the ledger for a moment is the common holder, not only another process: looking once
/// refused valid results and discarded their copy for a brief contention. Past the
/// budget the ledger is unavailable for that call (the acceptance is refused and the result goes
/// to a discard job); the tick does not renew this lock-wait budget.
pub(in crate::scene_resources) const TICK_WAIT: std::time::Duration =
    std::time::Duration::from_millis(20);
const LOCK_POLL: std::time::Duration = std::time::Duration::from_micros(500);

/// Recovers a poisoned process-local mutex without discarding the persisted ledger.
fn relock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every change under these locks is one assignment: a panic leaves consistent data.
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// An owner as the ledger records it. `pid` is only for messages: liveness is the lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::scene_resources) struct Owner {
    pub pid: u32,
    pub since: u64,
}

/// An owner whose lease was gone when the ledger was opened, and was removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::scene_resources) struct Reaped {
    pub owner: u128,
    pub pid: u32,
    pub since: u64,
}

/// The lease of this process on one ledger: held while it has holders there.
#[derive(Debug)]
struct LiveLease {
    // Held, never read: dropping the file releases the lease.
    _file: File,
    path: PathBuf,
}

#[derive(Debug)]
struct Inner {
    dir: PathBuf,
    live: Mutex<Option<LiveLease>>,
}

static LEDGERS: LazyLock<Mutex<HashMap<PathBuf, Arc<Inner>>>> = LazyLock::new(Mutex::default);

/// The ledger of one workspace. Cheap to clone: every clone is the same ledger, and a process
/// has one lease per ledger however many results it holds.
#[derive(Debug, Clone)]
pub(in crate::scene_resources) struct Ledger(Arc<Inner>);

impl Ledger {
    /// The ledger of `workspace` (canonical when it exists). Creates nothing on disk.
    pub(in crate::scene_resources) fn of(workspace: &Path) -> Self {
        let canonical = fs::canonicalize(workspace).unwrap_or_else(|_| workspace.to_path_buf());
        let dir = canonical.join(DIR);
        let mut all = relock(&LEDGERS);
        let inner = all.entry(dir.clone()).or_insert_with(|| {
            Arc::new(Inner {
                dir,
                live: Mutex::new(None),
            })
        });
        Self(Arc::clone(inner))
    }

    /// The folder of the ledger (what two records of one workspace have in common).
    pub(in crate::scene_resources) fn dir(&self) -> &Path {
        &self.0.dir
    }

    /// Opens what exists. A workspace with no ledger yields a tracker with nothing in it.
    pub(in crate::scene_resources) fn lock(&self) -> Tracker {
        Tracker::open(&self.0, false, LOCK_WAIT)
    }

    /// [`Ledger::lock`] for a caller that must not block (the window's tick): it waits at most
    /// [`TICK_WAIT`]. When `ledger.lock` is still held after that (by another process, or by
    /// another job of this one) the tracker is faulted (`fault()` says it was held and for how
    /// long it was waited for) and nothing is read or changed, exactly like any other
    /// unavailable ledger.
    pub(in crate::scene_resources) fn try_lock(&self) -> Tracker {
        self.lock_for(TICK_WAIT)
    }

    /// [`Ledger::try_lock`] with the budget chosen by the caller (tests of the budget itself).
    pub(in crate::scene_resources) fn lock_for(&self, wait: std::time::Duration) -> Tracker {
        Tracker::open(&self.0, false, wait)
    }

    /// Opens the ledger, creating it when it is not there yet: for whoever is about to claim.
    pub(in crate::scene_resources) fn lock_creating(&self) -> Tracker {
        Tracker::open(&self.0, true, LOCK_WAIT)
    }
}

/// The ledger, locked: no other process or thread changes it while this lives. Changes are
/// written when each method ends. Hold it only across short, non-blocking work (no hashing, no
/// waiting for a lease): the only file operation done under it is creating or removing one
/// folder, which is what keeps "the folder exists" and "its entry exists" true together.
#[derive(Debug)]
pub(in crate::scene_resources) struct Tracker {
    inner: Arc<Inner>,
    /// `None` when the workspace has no ledger (nothing is claimed) or it could not be opened.
    lock: Option<DiskLock>,
    owners: BTreeMap<u128, Owner>,
    entries: BTreeMap<PathBuf, Entry>,
    reaped: Vec<Reaped>,
    fault: Option<String>,
    /// Entries changed since they were written.
    dirty: bool,
}

#[derive(Debug)]
struct DiskLock {
    // Held, never read: dropping the file unlocks the ledger.
    _file: File,
    path: PathBuf,
}

impl Tracker {
    /// Acquires coordination, validates state and reaps dead leases; any failure stays a fault.
    fn open(inner: &Arc<Inner>, create: bool, wait: std::time::Duration) -> Self {
        let mut tracker = Self {
            inner: Arc::clone(inner),
            lock: None,
            owners: BTreeMap::new(),
            entries: BTreeMap::new(),
            reaped: Vec::new(),
            fault: None,
            dirty: false,
        };
        match DiskLock::acquire(&inner.dir, create, wait) {
            Ok(Some(lock)) => {
                tracker.lock = Some(lock);
                match tracker.load() {
                    Ok(()) => {
                        tracker.reap();
                        tracker.flush();
                    }
                    Err(why) => tracker.fault = Some(why),
                }
            }
            Ok(None) => {}
            Err(why) => tracker.fault = Some(why),
        }
        tracker
    }

    /// Why the ledger cannot be trusted right now, if it cannot. While this is set claims are not
    /// recorded and nothing is reported as removable.
    pub(in crate::scene_resources) fn fault(&self) -> Option<&str> {
        self.fault.as_deref()
    }

    /// The owners whose lease was gone when this was opened (already removed).
    pub(in crate::scene_resources) fn reaped(&self) -> &[Reaped] {
        &self.reaped
    }

    /// Owners that are alive: every other one was reaped when the ledger was opened.
    pub(in crate::scene_resources) fn owners(&self) -> impl Iterator<Item = (u128, &Owner)> {
        self.owners.iter().map(|(token, owner)| (*token, owner))
    }

    /// Exposes the locked snapshot; callers must check fault before relying on ownership.
    pub(in crate::scene_resources) fn entries(&self) -> impl Iterator<Item = (&Path, &Entry)> {
        self.entries
            .iter()
            .map(|(key, entry)| (key.as_path(), entry))
    }

    /// Counts entries within a resource root, including the root itself when registered.
    pub(in crate::scene_resources) fn count_under(&self, root: &Path) -> usize {
        self.entries
            .keys()
            .filter(|key| key.starts_with(root))
            .count()
    }

    /// Whether some entry lies below the folder `key`.
    pub(in crate::scene_resources) fn has_below(&self, key: &Path) -> bool {
        self.entries
            .keys()
            .any(|other| other != key && other.starts_with(key))
    }

    /// `holder` created the file at `key`, published under the lease of `root`.
    ///
    /// `Ok` means the claim is **on disk**: it was written (or the ledger already said so) by the
    /// time this answers. `Err` is the reason it is not, and then nothing was recorded: the
    /// caller owns a file the ledger knows nothing about and must not report success for it.
    pub(in crate::scene_resources) fn create_file(
        &mut self,
        holder: HolderId,
        key: PathBuf,
        root: &Path,
        stamp: Option<Stamp>,
    ) -> Result<(), String> {
        self.insert(holder, key, Item::File(stamp), Some(root.to_path_buf()))
    }

    /// `holder` created the folder at `key`. Answers like [`Tracker::create_file`].
    pub(in crate::scene_resources) fn create_dir(
        &mut self,
        holder: HolderId,
        key: PathBuf,
    ) -> Result<(), String> {
        self.insert(holder, key, Item::Dir, None)
    }

    // A stale entry at a path that was just created cannot be a live holder's: whoever listed it
    // removed or lost the file first. The new entry replaces it.
    /// Replaces a stale publication entry and persists the new holder before success.
    fn insert(
        &mut self,
        holder: HolderId,
        key: PathBuf,
        item: Item,
        root: Option<PathBuf>,
    ) -> Result<(), String> {
        if let Some(why) = &self.fault {
            return Err(why.clone());
        }
        if self.lock.is_none() {
            return Err(format!(
                "the claims ledger `{}` is not open",
                self.inner.dir.display()
            ));
        }
        if key.to_str().is_none() {
            // The ledger is JSON text: a path that is not text cannot be vouched for, and a
            // claim that is not recorded would leave a file nobody can ever take back.
            return Err(format!(
                "`{}` cannot be written in the claims ledger (not valid UTF-8)",
                key.display()
            ));
        }
        self.entries.insert(
            key,
            Entry {
                item,
                root,
                holders: BTreeSet::from([holder]),
            },
        );
        self.dirty = true;
        self.flush();
        // `flush` is what writes: a fault now means the entry above is in memory only.
        match &self.fault {
            Some(why) => Err(why.clone()),
            None => Ok(()),
        }
    }

    /// `holder` relies on the entry at `key`, if a pending result created one. Answers whether
    /// there was one.
    pub(in crate::scene_resources) fn join(&mut self, holder: HolderId, key: &Path) -> bool {
        if self.fault.is_some() {
            return false;
        }
        let Some(entry) = self.entries.get_mut(key) else {
            return false;
        };
        entry.holders.insert(holder);
        self.dirty = true;
        self.flush();
        self.fault.is_none()
    }

    /// `holder` gives up the entry at `key` without accepting.
    pub(in crate::scene_resources) fn leave(&mut self, holder: HolderId, key: &Path) -> Leave {
        if let Some(why) = &self.fault {
            return Leave::Unverifiable(why.clone());
        }
        let Some(entry) = self.entries.get_mut(key) else {
            // Nothing to change, but this process may hold no entry any more: let go of its lease.
            self.flush();
            return Leave::Untracked;
        };
        entry.holders.remove(&holder);
        self.dirty = true;
        if !entry.holders.is_empty() {
            self.flush();
            return self.unless_unwritten(Leave::Shared);
        }
        let leave = match self.entries.remove(key) {
            Some(entry) => Leave::Last(entry.item),
            None => Leave::Untracked,
        };
        self.flush();
        self.unless_unwritten(leave)
    }

    /// Like [`Tracker::leave`], but when `holder` is the **last** one the entry is **not**
    /// removed: the answer `Last` is given with the claim (and so this process's lease) still on
    /// disk. The caller proves and removes the file, then calls [`Tracker::leave`] again to drop
    /// the entry. A process that dies in between (`SIGKILL` during the hash of a large file) leaves
    /// an orphan entry that `recover` takes back, instead of a file nobody lists. The entry is not
    /// changed for `Last`, but the ledger is **rewritten as it is**: a ledger that cannot be
    /// written answers `Unverifiable` here, before anything is removed, exactly as `leave` does
    /// (fail closed). A holder that is not the last one is removed at once, as in `leave`.
    pub(in crate::scene_resources) fn begin_leave(
        &mut self,
        holder: HolderId,
        key: &Path,
    ) -> Leave {
        if let Some(why) = &self.fault {
            return Leave::Unverifiable(why.clone());
        }
        match self.entries.get(key) {
            Some(entry) if !entry.holders.iter().any(|other| *other != holder) => {
                let item = entry.item.clone();
                self.dirty = true;
                self.flush();
                self.unless_unwritten(Leave::Last(item))
            }
            _ => self.leave(holder, key),
        }
    }

    /// `holder` gives up the entry at `key` but the entry stays even when nobody else holds it:
    /// the file could not be removed now (its root is busy), so it must stay claimed, as an
    /// orphan, for [`recover`](super::super::recover) instead of being forgotten.
    pub(in crate::scene_resources) fn abandon(&mut self, holder: HolderId, key: &Path) -> Leave {
        if let Some(why) = &self.fault {
            return Leave::Unverifiable(why.clone());
        }
        let Some(entry) = self.entries.get_mut(key) else {
            self.flush();
            return Leave::Untracked;
        };
        entry.holders.remove(&holder);
        let leave = if entry.holders.is_empty() {
            Leave::Abandoned
        } else {
            Leave::Shared
        };
        self.dirty = true;
        self.flush();
        self.unless_unwritten(leave)
    }

    /// An answer that lets the caller remove or forget something is only given when the change
    /// behind it reached the disk. Otherwise the ledger still lists the entry (with this holder),
    /// so removing the file now would leave a claim for a file that is gone, and keeping the
    /// answer would let a later reader trust a state that was never written: the caller is told
    /// to keep what it has instead, and the entry stays for `recover`.
    fn unless_unwritten(&self, leave: Leave) -> Leave {
        match (&self.fault, &leave) {
            (Some(why), Leave::Last(_) | Leave::Abandoned | Leave::Shared) => {
                Leave::Unverifiable(why.clone())
            }
            _ => leave,
        }
    }

    /// The result that listed `keys` was accepted: the project owns them now, for every holder.
    ///
    /// `Ok` means the entries are **gone from the disk**. `Err` is the reason they may not be:
    /// the caller must not tell anyone the result was accepted (the entries stay claimed by this
    /// process, so nothing removes them while it lives). What a failed answer cannot say is
    /// whether the write reached the disk; a caller that discards after it is still safe, because
    /// a discard only removes files whose entry it still finds, so a leak is the worst outcome.
    pub(in crate::scene_resources) fn accept<'a>(
        &mut self,
        keys: impl Iterator<Item = &'a Path>,
    ) -> Result<(), String> {
        self.guard()?;
        for key in keys {
            self.dirty |= self.entries.remove(key).is_some();
        }
        self.flush();
        self.guard()
    }

    /// Merges the holder `from` into `to` for the entries at `keys` (two results of one
    /// operation becoming one). `Ok` only when the merge is on disk: on `Err` the caller keeps
    /// both lists as they were, so the entries stay held by `from` (or by `to`, when the write
    /// did happen) and nobody loses ownership.
    pub(in crate::scene_resources) fn transfer<'a>(
        &mut self,
        from: HolderId,
        to: HolderId,
        keys: impl Iterator<Item = &'a Path>,
    ) -> Result<(), String> {
        self.guard()?;
        for key in keys {
            if let Some(entry) = self.entries.get_mut(key)
                && entry.holders.remove(&from)
            {
                entry.holders.insert(to);
                self.dirty = true;
            }
        }
        self.flush();
        self.guard()
    }

    /// The fault of this tracker as an error, `Ok` when it can be trusted.
    fn guard(&self) -> Result<(), String> {
        match &self.fault {
            Some(why) => Err(why.clone()),
            None => Ok(()),
        }
    }

    /// Drops the entry at `key` whatever its holders: recovery decided about it.
    pub(in crate::scene_resources) fn forget(&mut self, key: &Path) -> Result<(), String> {
        self.guard()?;
        self.dirty |= self.entries.remove(key).is_some();
        self.flush();
        self.guard()
    }

    // --- persistence ---------------------------------------------------------------------

    /// Validates the persisted version and every entry before allowing any mutation.
    fn load(&mut self) -> Result<(), String> {
        let path = self.inner.dir.join(STATE);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("`{}`: {error}", path.display())),
        };
        let bad = |why: &str| format!("`{}` is not a claims ledger: {why}", path.display());
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|error| bad(&error.to_string()))?;
        match value.get("version").and_then(Value::as_u64) {
            Some(VERSION) => {}
            Some(other) => {
                return Err(format!(
                    "`{}` was written by a newer Tessel (ledger version {other})",
                    path.display()
                ));
            }
            None => return Err(bad("no version")),
        }
        for (token, owner) in object(&value, "owners").ok_or_else(|| bad("no owners"))? {
            let token = parse_owner(token).ok_or_else(|| bad("an owner token"))?;
            self.owners.insert(
                token,
                Owner {
                    pid: owner
                        .get("pid")
                        .and_then(Value::as_u64)
                        .and_then(|pid| u32::try_from(pid).ok())
                        .unwrap_or_default(),
                    since: owner
                        .get("since")
                        .and_then(Value::as_u64)
                        .unwrap_or_default(),
                },
            );
        }
        for (key, entry) in object(&value, "entries").ok_or_else(|| bad("no entries"))? {
            let entry = entry_from_json(entry).ok_or_else(|| bad("an entry"))?;
            self.entries.insert(PathBuf::from(key), entry);
        }
        Ok(())
    }

    /// Removes the owners whose lease is gone, with their claims, and any lease file nobody
    /// listed. Anything left holderless is an orphan, and stays.
    fn reap(&mut self) {
        let me = process_owner();
        let dead: Vec<u128> = self
            .owners
            .keys()
            .copied()
            .filter(|owner| *owner != me && !is_alive(&self.inner.dir, *owner))
            .collect();
        let mut changed = false;
        for owner in &dead {
            if let Some(record) = self.owners.remove(owner) {
                self.reaped.push(Reaped {
                    owner: *owner,
                    pid: record.pid,
                    since: record.since,
                });
            }
            for entry in self.entries.values_mut() {
                entry.holders.retain(|holder| holder.owner() != *owner);
            }
            let _ = fs::remove_file(live_path(&self.inner.dir, *owner));
            changed = true;
        }
        self.sweep_unlisted_leases();
        self.dirty |= changed;
    }

    /// A lease file that is not listed and can be taken belongs to an owner that died between
    /// creating it and being written down (both happen in one critical section, so a live owner
    /// never has one).
    fn sweep_unlisted_leases(&self) {
        let me = process_owner();
        let Ok(names) = fs::read_dir(self.inner.dir.join(LIVE)) else {
            return;
        };
        for name in names.flatten() {
            let Some(owner) = name.file_name().to_str().and_then(parse_owner) else {
                continue;
            };
            if owner != me && !self.owners.contains_key(&owner) && !is_alive(&self.inner.dir, owner)
            {
                let _ = fs::remove_file(name.path());
            }
        }
    }

    /// Keeps the owners in step with the entries: this process's lease exists while a holder of
    /// this process is listed (taken before the entries naming it are written). Answers whether
    /// the list of owners changed and whether the lease is no longer needed.
    ///
    /// The lease is **not** let go of here: it is released by [`Tracker::flush`] only once the
    /// ledger without this process has been written. Releasing it first would let any other
    /// process reap this one as dead while the ledger on disk (the write failed) still lists its
    /// claims, and `recover` would then remove what a live process, or an accepted result, owns.
    fn sync_lease(&mut self) -> Result<(bool, bool), String> {
        let me = process_owner();
        let before = self.owners.len();
        let holding: BTreeSet<u128> = self
            .entries
            .values()
            .flat_map(|entry| entry.holders.iter().map(HolderId::owner))
            .collect();
        // Another owner that is alive but holds nothing: its own process lets its lease go the
        // next time it looks, and lists itself again if it claims something.
        self.owners
            .retain(|owner, _| *owner == me || holding.contains(owner));
        let mut live = relock(&self.inner.live);
        let mut changed = self.owners.len() != before;
        let mut release = false;
        if holding.contains(&me) {
            if live.is_none() {
                *live = Some(take_lease(&self.inner.dir, me)?);
            }
            if let btree_map::Entry::Vacant(slot) = self.owners.entry(me) {
                slot.insert(Owner {
                    pid: std::process::id(),
                    since: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_or(0, |elapsed| elapsed.as_secs()),
                });
                changed = true;
            }
        } else {
            release = live.is_some();
            changed |= self.owners.remove(&me).is_some();
        }
        Ok((changed, release))
    }

    /// Lets go of this process's lease (it holds nothing in this ledger any more).
    fn release_lease(&self) {
        if let Some(lease) = relock(&self.inner.live).take() {
            let _ = fs::remove_file(&lease.path);
        }
    }

    /// Writes the ledger when something changed (or removes the file when it is empty). A
    /// failure marks the tracker faulty: what it still answers is "kept", never "removable", and
    /// the lease stays, because the disk still lists this process's claims.
    fn flush(&mut self) {
        if self.lock.is_none() || self.fault.is_some() {
            return;
        }
        let result = self.sync_lease().and_then(|(changed, release)| {
            if self.dirty || changed {
                self.write()?;
            }
            if release {
                self.release_lease();
            }
            Ok(())
        });
        self.dirty = false;
        if let Err(why) = result {
            self.fault = Some(why);
        }
    }

    /// Atomically replaces ledger JSON, or removes it only when no entries or owners remain.
    fn write(&self) -> Result<(), String> {
        let state = self.inner.dir.join(STATE);
        if self.entries.is_empty() && self.owners.is_empty() {
            return match fs::remove_file(&state) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => {
                    Err(format!("`{}`: {error}", state.display()))
                }
                _ => Ok(()),
            };
        }
        let mut owners = Map::new();
        for (token, owner) in &self.owners {
            owners.insert(
                owner_text(*token),
                json!({ "pid": owner.pid, "since": owner.since }),
            );
        }
        let mut entries = Map::new();
        for (key, entry) in &self.entries {
            if let Some(text) = key.to_str() {
                entries.insert(text.to_string(), entry_to_json(entry));
            }
        }
        let document = json!({ "version": VERSION, "owners": owners, "entries": entries });
        let tmp = self.inner.dir.join(STATE_TMP);
        let result = (|| {
            let mut file = File::create(&tmp)?;
            file.write_all(&serde_json::to_vec(&document).map_err(io::Error::other)?)?;
            file.flush()?;
            fs::rename(&tmp, &state)
        })();
        result.map_err(|error| {
            let _ = fs::remove_file(&tmp);
            format!("`{}`: {error}", state.display())
        })
    }
}

impl Drop for Tracker {
    /// A ledger with nothing in it is removed, so a workspace without pending results looks like
    /// one that never had any. Done under the lock, before releasing it.
    fn drop(&mut self) {
        let Some(lock) = self.lock.take() else {
            return;
        };
        if self.fault.is_none() && self.entries.is_empty() && self.owners.is_empty() {
            let dir = &self.inner.dir;
            let _ = fs::remove_file(dir.join(STATE));
            let _ = fs::remove_file(dir.join(STATE_TMP));
            // A lease file still there is of an owner that is alive but holds nothing yet let go
            // of it; its next look removes it and then the folder. Until then the ledger stays.
            let leases_gone = match fs::remove_dir(dir.join(LIVE)) {
                Ok(()) => true,
                Err(error) => error.kind() == io::ErrorKind::NotFound,
            };
            if leases_gone {
                // Unlink first, unlock after (the lock file is unlinked while held): a waiter
                // that locked the old file sees the mismatch and starts again.
                let _ = fs::remove_file(&lock.path);
                let _ = fs::remove_dir(dir);
            }
        }
        drop(lock);
    }
}

impl DiskLock {
    /// Takes the ledger lock, waiting for it up to `wait`. A holder may keep it briefly or
    /// across several writes during a healthy recovery; `Duration::ZERO` looks once and never waits. `Ok(None)` when there is
    /// no ledger and `create` is not set.
    fn acquire(
        dir: &Path,
        create: bool,
        wait: std::time::Duration,
    ) -> Result<Option<Self>, String> {
        let path = dir.join(LOCK);
        let mut dangling = 0;
        let deadline = Instant::now() + wait;
        for _ in 0..ATTEMPTS {
            if create {
                match fs::create_dir(dir) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    // A folder another process is removing right now can answer "not found" to
                    // a create; a workspace that is really gone answers it every time.
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        std::thread::yield_now();
                        continue;
                    }
                    Err(error) => return Err(format!("`{}`: {error}", dir.display())),
                }
            }
            let file = match OpenOptions::new()
                .read(true)
                .write(true)
                .create(create)
                .truncate(false)
                .open(&path)
            {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::NotFound && !create => {
                    // No lock file normally means no ledger. A state file without its lock is
                    // not "empty": it lists claims nobody can serialise changes to.
                    match look_for_damage(dir, &mut dangling) {
                        Look::Empty => return Ok(None),
                        Look::Again => {
                            std::thread::sleep(DANGLING_PAUSE);
                            continue;
                        }
                        Look::Damaged => return Err(damaged(dir)),
                    }
                }
                // The folder was removed between creating it and opening the lock: again.
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                // macOS answers EINVAL to an open that races with the unlink of the same lock
                // file by the process that emptied the ledger: the same case, retried.
                Err(error) if error.kind() == io::ErrorKind::InvalidInput => {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                }
                Err(error) => return Err(format!("open `{}`: {error}", path.display())),
            };
            // Bounded (BL112.3): healthy recovery can hold this lock across several full ledger
            // rewrites. A stopped (`SIGSTOP`) or stuck holder can keep it longer, and a caller
            // that waits forever is a stuck caller. The window's tick (where `accept` runs)
            // waits `TICK_WAIT` only. Past the bound the ledger is reported unavailable,
            // which every caller already handles by failing closed: an acceptance is refused (the
            // result stays claimed and goes to a discard job), a removal keeps the file, a
            // recovery fails with its partial report. The message does not say who holds it: another process
            // and another job of this one look the same.
            match lock_within(&file, deadline) {
                Ok(true) => {}
                Ok(false) => {
                    return Err(format!(
                        "`{}` is held right now by a holder that did not let go of it; gave up after {:?}",
                        path.display(),
                        wait
                    ));
                }
                // A lock file that was unlinked and is being reclaimed can refuse (EINVAL on
                // macOS): it is the same case as one that turned out to be unlinked.
                Err(error) if error.kind() == io::ErrorKind::InvalidInput => {
                    std::thread::yield_now();
                    continue;
                }
                Err(error) => return Err(format!("lock `{}`: {error}", path.display())),
            }
            if same_file(&path, &file) {
                return Ok(Some(Self { _file: file, path }));
            }
        }
        Err(format!(
            "`{}` is being created and removed too quickly to be locked",
            dir.display()
        ))
    }
}

/// Takes the exclusive lock of `file`, waiting until `deadline`. `Ok(false)` when it was still
/// held by then.
fn lock_within(file: &File, deadline: Instant) -> io::Result<bool> {
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(true),
            Err(TryLockError::WouldBlock) => {
                if Instant::now() >= deadline {
                    return Ok(false);
                }
                std::thread::sleep(LOCK_POLL);
            }
            Err(TryLockError::Error(error)) => return Err(error),
        }
    }
}

enum Look {
    /// No lock and no state: there is no ledger.
    Empty,
    /// The state is there without the lock, but not for long enough to call it damage.
    Again,
    Damaged,
}

/// Whether the lock is missing **and** the state file is there, on `DANGLING_SEEN` looks in a
/// row (`seen` counts them). Both legitimate races are one look long: a creator that makes the
/// lock a moment after this looked, and the process that emptied the ledger removing the state
/// before the lock. A ledger really missing its lock stays that way, and then the answer is a
/// fault (fail closed), never "empty".
fn look_for_damage(dir: &Path, seen: &mut usize) -> Look {
    if fs::symlink_metadata(dir.join(STATE)).is_err() {
        *seen = 0;
        return Look::Empty;
    }
    *seen += 1;
    if *seen < DANGLING_SEEN {
        Look::Again
    } else if fs::symlink_metadata(dir.join(LOCK)).is_err() {
        Look::Damaged
    } else {
        *seen = 0;
        Look::Again
    }
}

/// Reports a persistent state file with no coordination lock as damage, never as empty.
fn damaged(dir: &Path) -> String {
    format!(
        "`{}` exists but its `{LOCK}` is missing: the claims ledger is damaged (the lock was deleted); nothing was changed, restore `{LOCK}` or remove `{DIR}` once no Tessel is running",
        dir.join(STATE).display()
    )
}

// --- leases ------------------------------------------------------------------------------

/// Locates a random process token lease; PIDs never decide ownership.
fn live_path(dir: &Path, owner: u128) -> PathBuf {
    dir.join(LIVE).join(owner_text(owner))
}

/// Takes this process's lease on the ledger: a file nobody else can lock while this lives.
fn take_lease(dir: &Path, owner: u128) -> Result<LiveLease, String> {
    let folder = dir.join(LIVE);
    match fs::create_dir(&folder) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(format!("`{}`: {error}", folder.display())),
    }
    let path = live_path(dir, owner);
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .map_err(|error| format!("`{}`: {error}", path.display()))?;
    file.try_lock()
        .map_err(|error| format!("`{}`: {error:?}", path.display()))?;
    // Informational: who to name in a message. Never what decides that the owner is alive.
    let _ = write!(file, "{}", std::process::id());
    Ok(LiveLease { _file: file, path })
}

/// Whether the owner's lease is still held. Missing file: it is gone. Cannot tell (an error
/// other than "held"): alive, because removing a live process's claims is the unsafe answer.
fn is_alive(dir: &Path, owner: u128) -> bool {
    let path = live_path(dir, owner);
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return false,
        Err(_) => return true,
    };
    match file.try_lock_shared() {
        // Nobody holds it exclusively: the owner is gone. (Dropping `file` releases ours.)
        Ok(()) => false,
        Err(TryLockError::WouldBlock) => true,
        Err(TryLockError::Error(_)) => true,
    }
}

#[cfg(unix)]
/// Checks whether the open lock still names the path, detecting unlink and replacement races.
fn same_file(path: &Path, file: &File) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (fs::metadata(path), file.metadata()) {
        (Ok(on_disk), Ok(open)) => on_disk.dev() == open.dev() && on_disk.ino() == open.ino(),
        _ => false,
    }
}

/// Without device and inode numbers the check cannot be made; the ledger then protects only
/// against processes that never remove it (limit documented for non-Unix platforms).
#[cfg(not(unix))]
fn same_file(path: &Path, _file: &File) -> bool {
    path.exists()
}

// --- format ------------------------------------------------------------------------------

/// Encodes the random process token in the stable ledger key format.
fn owner_text(owner: u128) -> String {
    format!("{owner:032x}")
}

/// Rejects malformed persisted owner tokens instead of inventing a process identity.
fn parse_owner(text: &str) -> Option<u128> {
    (text.len() == 32).then(|| u128::from_str_radix(text, 16).ok())?
}

/// Combines process token and per-result sequence into an unambiguous persisted holder.
fn holder_text(holder: &HolderId) -> String {
    format!("{}:{}", owner_text(holder.owner()), holder.seq())
}

/// Requires both a valid process token and result sequence to accept a holder key.
fn parse_holder(text: &str) -> Option<HolderId> {
    let (owner, seq) = text.split_once(':')?;
    Some(HolderId::new(parse_owner(owner)?, seq.parse().ok()?))
}

/// Requires an object field when validating persisted state; other JSON types are rejected.
fn object<'a>(value: &'a Value, key: &str) -> Option<&'a Map<String, Value>> {
    value.get(key)?.as_object()
}

/// Encodes the publication identity, root and complete set of holders.
fn entry_to_json(entry: &Entry) -> Value {
    let holders: Vec<String> = entry.holders.iter().map(holder_text).collect();
    match &entry.item {
        Item::Dir => json!({ "kind": "dir", "holders": holders }),
        Item::File(stamp) => json!({
            "kind": "file",
            "root": entry.root.as_deref().and_then(Path::to_str),
            "stamp": stamp.as_ref().map(stamp_to_json),
            "holders": holders,
        }),
    }
}

/// Rejects malformed entry identities or holders rather than silently dropping ownership.
fn entry_from_json(value: &Value) -> Option<Entry> {
    let holders = value
        .get("holders")?
        .as_array()?
        .iter()
        .map(|holder| parse_holder(holder.as_str()?))
        .collect::<Option<BTreeSet<_>>>()?;
    let (item, root) = match value.get("kind")?.as_str()? {
        "dir" => (Item::Dir, None),
        "file" => {
            let stamp = match value.get("stamp")? {
                Value::Null => None,
                stamp => Some(stamp_from_json(stamp)?),
            };
            let root = value.get("root").and_then(Value::as_str).map(PathBuf::from);
            (Item::File(stamp), root)
        }
        _ => return None,
    };
    Some(Entry {
        item,
        root,
        holders,
    })
}

/// Persists content identity and the platform file identity needed for safe cleanup.
fn stamp_to_json(stamp: &Stamp) -> Value {
    #[cfg(unix)]
    {
        json!({ "len": stamp.len, "device": stamp.device, "inode": stamp.inode,
            "sha256": stamp.sha256 })
    }
    #[cfg(not(unix))]
    {
        json!({ "len": stamp.len, "sha256": stamp.sha256 })
    }
}

/// Requires every platform identity field and content hash before accepting a saved stamp.
fn stamp_from_json(value: &Value) -> Option<Stamp> {
    Some(Stamp {
        len: value.get("len")?.as_u64()?,
        #[cfg(unix)]
        device: value.get("device")?.as_u64()?,
        #[cfg(unix)]
        inode: value.get("inode")?.as_u64()?,
        sha256: value.get("sha256")?.as_str()?.to_string(),
    })
}
