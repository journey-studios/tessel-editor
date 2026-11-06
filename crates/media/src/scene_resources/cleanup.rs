//! Ownership of what an adoption or an import wrote into the workspace (BL089, BL096, BL097), so a
//! result that is cancelled, rejected, late or discarded can take back exactly its own files and
//! nothing else.
//!
//! [`Created`] is a *record*: the files this operation published and the folders it made, in
//! creation order, plus what it merely **relies on** (a file another pending result copied and it
//! reused). [`Provisional`] is the *owner* of a record while the result is not yet accepted: it
//! removes the record when dropped unless the result was accepted ([`Provisional::accept`]).
//! Separating the two is deliberate:
//!
//! * A result that crosses a job slot, a `Pending` queue or a panic unwinding is always cleaned
//!   up by the destructor, so no new code path can forget to discard it.
//! * The owner is **not `Clone`**. A copy would drop later and delete the files of the consumer
//!   that kept the original. Whoever needs two views of one result shares a reference to it.
//!
//! # Shared files (BL096)
//!
//! Two results into the same scene folder share every file the second one reuses. Each record is
//! a *holder* in the process-wide [`claims`](super::claims) registry; discarding a record removes
//! a file only when it is the **last** holder of a file a pending result created, so:
//!
//! * discarding the first result never deletes what an accepted (or still pending) second one
//!   reused, in any order of accept and discard;
//! * the last one to leave takes everything back, including the folders the first one made;
//! * a file that was in the workspace before, or that an accepted result owns, has no entry and
//!   is never removed by anyone.
//!
//! Accepting is [`Provisional::accept`]: explicit and **fallible**. Its entries are deleted from
//! the on-disk ledger, which is what turns the files into the project's, and it answers `Ok` only
//! once that is written. The flow that applies a result calls it as its last step and undoes
//! itself when it fails (the result then stays pending and claimed). Merging two results
//! ([`Provisional::absorb`]) writes the transfer first and moves nothing when it fails. The
//! registry is the workspace's on-disk ledger (see [`claims`](super::claims)), so another process
//! reusing the same file is covered too.
//!
//! # A claim that cannot be written (BL111)
//!
//! A record never owns what the ledger does not list. [`Created::published`] answers success only
//! when every claim it needed is on disk. When one cannot be written the copy is already there,
//! so it is not a refusal "before copying": the file is verified (identity and SHA-256, like any
//! removal) and taken back at once, together with the still-empty folders made for it, and the
//! caller gets an [`Unclaimed`] saying what was and was not. That is only safe because the caller
//! holds the lease of the root, the file was published a moment ago and the ledger never listed
//! it, so no other result can have joined it. What cannot be taken back is reported and left,
//! never claimed.
//!
//! # What is never removed
//!
//! * a file or folder the operation did not create and no pending result holds;
//! * a file that is no longer the one the operation published (BL097): replaced by another file
//!   (different inode), grown or shortened, **or rewritten in place** (same inode, same length,
//!   other content). The proof is the file's own identity *and* its SHA-256, the hash the bundle
//!   declares and `materialize` verified before publishing; a file that cannot be read to check
//!   it is kept (fail closed). It is reported as `kept`;
//! * a recorded folder that still has content (`remove_dir`, never `remove_dir_all`).
//!
//! Limit, stated honestly: the check is by content, so a rewrite that stores the very same bytes
//! is indistinguishable from the published file and is removed (nothing the writer wrote is
//! lost). Between the last check and the unlink a writer could still win (BL079): this is a
//! cooperative contract, not a defense against a hostile writer of the same user.
//!
//! # Cost
//!
//! Verifying reads each file to remove once (SHA-256 streaming, the same pass the copy made). It
//! only runs where a file is actually removed, never for a result that is accepted, and a
//! cancelled import has already rolled back most of its files before this runs. A destructor
//! that drops a large result therefore pays about as much as copying it again; callers that must
//! not block (a UI thread) discard from a job (`job.discard`, BL094): `MediaHost` moves the result
//! into a `discard` job, which keeps this owner (and its claims) until it has finished.
//!
//! Removal first takes the same cooperative [`RootLease`] the publication used, so it cannot
//! interleave with another Tessel operation working in that root. A root that stays busy is
//! reported as `kept`, not forced.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde_json::{Value, json};
use tessel_editor_core::CommandError;
use tessel_editor_core::jobs::{CancelToken, JobError};
use tessel_editor_core::scene_resources::ResourcePath;

use super::claims::{self, HolderId, Item, Leave, Ledger, Stamp, Tracker};
use crate::directed_resources::RootLease;
use crate::fsx::hash_file;

/// How long removal waits for a root another operation holds, before keeping the files and
/// saying so. Bounded because this can run in a destructor.
const LEASE_ATTEMPTS: usize = 8;
const LEASE_PAUSE: Duration = Duration::from_millis(25);

impl Stamp {
    /// Only a regular file (not a link) has a stamp. `sha256` is the content it was verified to
    /// have when it was published.
    fn of(path: &Path, sha256: &str) -> Option<Self> {
        let metadata = fs::symlink_metadata(path).ok()?;
        if !metadata.is_file() {
            return None;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Some(Self {
                len: metadata.len(),
                device: metadata.dev(),
                inode: metadata.ino(),
                sha256: sha256.to_string(),
            })
        }
        #[cfg(not(unix))]
        {
            // No device/inode numbers: length and content are the identity available (limit
            // shared with the lease, see `lease.rs`).
            Some(Self {
                len: metadata.len(),
                sha256: sha256.to_string(),
            })
        }
    }

    /// Whether `seen` is the published file by its cheap identity. Content is checked after.
    fn is_the_file_of(&self, seen: &Observed) -> bool {
        #[cfg(unix)]
        {
            self.len == seen.len && self.device == seen.device && self.inode == seen.inode
        }
        #[cfg(not(unix))]
        {
            self.len == seen.len
        }
    }
}

/// What a file looks like right now, including the times a write moves: comparing two
/// observations around the read proves nothing wrote to the file while it was being checked.
#[derive(Debug, PartialEq, Eq)]
struct Observed {
    len: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    changed: (i64, i64),
}

impl Observed {
    /// `Ok(None)` when the path is not a regular file (a link, a folder).
    fn of(path: &Path) -> io::Result<Option<Self>> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_file() {
            return Ok(None);
        }
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Ok(Some(Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
            #[cfg(unix)]
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        }))
    }
}

/// A path as the caller spells it (for messages and file operations) and its registry key.
#[derive(Debug)]
struct Owned {
    path: PathBuf,
    key: PathBuf,
    /// The ledger of the workspace this belongs to: where its claim lives.
    ledger: Ledger,
    /// `false` for something the record only relies on (reused from another pending result).
    created: bool,
}

/// Files and folders one operation created, in creation order, and what it relies on that
/// another pending result created. Plain data plus a place in the claims registry: cloning it is
/// not possible on purpose (see the module docs); move it into a [`Provisional`] or call
/// [`Created::remove`]. Dropping a bare record accepts it best effort (see the `Drop` impl).
#[derive(Debug)]
pub struct Created {
    holder: HolderId,
    /// Where new claims go: the ledger of the workspace this record writes into, set by
    /// [`Created::bind`] before the first one.
    ledger: Option<Ledger>,
    /// Roots whose lease removal takes first: where `files` were published or reused.
    roots: Vec<PathBuf>,
    files: Vec<Owned>,
    dirs: Vec<Owned>,
}

impl Default for Created {
    fn default() -> Self {
        Self {
            holder: claims::next_holder(),
            ledger: None,
            roots: Vec::new(),
            files: Vec::new(),
            dirs: Vec::new(),
        }
    }
}

impl Created {
    /// Nothing was created (reusing what another result copied does not count).
    pub fn is_empty(&self) -> bool {
        self.files().next().is_none() && self.dirs().next().is_none()
    }

    /// Files the operation published, in publication order.
    pub fn files(&self) -> impl Iterator<Item = &Path> {
        created(&self.files)
    }

    /// Folders the operation made, in creation order (a parent before its children).
    pub fn dirs(&self) -> impl Iterator<Item = &Path> {
        created(&self.dirs)
    }

    /// Files and folders this record only relies on, because another pending result created
    /// them and this one reused them. They stay while any holder is left.
    pub fn relied_on(&self) -> impl Iterator<Item = &Path> {
        self.files
            .iter()
            .chain(&self.dirs)
            .filter(|owned| !owned.created)
            .map(|owned| owned.path.as_path())
    }

    /// Binds this record to the claims ledger of `workspace`, where its claims are kept (BL111).
    /// Every record that creates or joins something is bound first; a record that is not has
    /// nothing to claim. A second call with another workspace is ignored: one record writes into
    /// one workspace.
    pub(super) fn bind(&mut self, workspace: &Path) -> Ledger {
        self.ledger
            .get_or_insert_with(|| Ledger::of(workspace))
            .clone()
    }

    /// The workspace ledger chosen on first bind; an unbound record owns no disk claims.
    fn ledger(&self) -> Option<&Ledger> {
        self.ledger.as_ref()
    }

    /// Records a folder this operation made, under the ledger lock the caller already holds
    /// (so the folder and its entry appear together). `Err` (the reason) means the claim is not
    /// on disk: the folder is then this record's to deal with, not an adopted result's, and it is
    /// **not** listed among the folders the record owns.
    pub(super) fn dir_with(
        &mut self,
        tracker: &mut Tracker,
        path: PathBuf,
        key: PathBuf,
    ) -> Result<(), String> {
        let Some(ledger) = self.ledger().cloned() else {
            return Ok(());
        };
        if self.has(&self.dirs, &key) {
            return Ok(());
        }
        tracker.create_dir(self.holder, key.clone())?;
        self.dirs.push(Owned {
            path,
            key,
            ledger,
            created: true,
        });
        Ok(())
    }

    /// Relies on a folder some pending result created, if one did. A folder nobody created is
    /// not recorded: nobody will remove it.
    pub(super) fn join_dir_with(
        &mut self,
        tracker: &mut Tracker,
        path: PathBuf,
        key: PathBuf,
    ) -> Result<(), JobError> {
        let Some(ledger) = self.ledger().cloned() else {
            return Ok(());
        };
        if self.has(&self.dirs, &key) {
            return Ok(());
        }
        let joined = tracker.join(self.holder, &key);
        if let Some(why) = tracker.fault() {
            return Err(claims_unavailable(why));
        }
        if joined {
            self.dirs.push(Owned {
                path,
                key,
                ledger,
                created: false,
            });
        }
        Ok(())
    }

    /// Persists a newly created directory before adding it to this record.
    fn dir(&mut self, path: PathBuf, key: PathBuf) -> Result<(), String> {
        let Some(ledger) = self.ledger().cloned() else {
            return Ok(());
        };
        let mut tracker = ledger.lock_creating();
        self.dir_with(&mut tracker, path, key)
    }

    /// Joins an existing directory once; repeated joins avoid taking the ledger lock.
    pub(super) fn join_dir(&mut self, path: PathBuf, key: PathBuf) -> Result<(), JobError> {
        if self.has(&self.dirs, &key) {
            return Ok(());
        }
        let Some(ledger) = self.ledger().cloned() else {
            return Ok(());
        };
        let mut tracker = ledger.lock();
        self.join_dir_with(&mut tracker, path, key)
    }

    /// Records the file just published at `relative` under `root` (`canonical_root` is the same
    /// folder resolved), with the SHA-256 it was verified to have, and the folders that had to be
    /// made for it (`new_dirs`, shallowest first: where each is spelled and its registry key).
    ///
    /// **Fails closed, after the copy.** When any of those claims cannot be written to the ledger
    /// the copy is already on disk, so the answer cannot be "refused before copying": the file is
    /// taken back at once (verified to be the one just published) and so are the folders made for
    /// it that are still empty, and the result says what was and was not. A failed proof or
    /// removal can leave an unclaimed file in `kept`; recovery cannot find that file by ledger.
    ///
    /// Safe to remove because of what the caller guarantees: it holds the lease of `root`, the
    /// file was published by this operation a moment ago (an adoption never overwrites), and the
    /// ledger never listed it, so no other result can have joined it (joining needs the entry).
    pub(super) fn published(
        &mut self,
        root: &Path,
        canonical_root: &Path,
        relative: &ResourcePath,
        sha256: &str,
        new_dirs: &[(PathBuf, PathBuf)],
    ) -> Result<(), Unclaimed> {
        if self.ledger().is_none() {
            return Ok(());
        }
        let (path, key) = locate(root, canonical_root, relative);
        let stamp = Stamp::of(&path, sha256);
        let claimed = new_dirs
            .iter()
            .try_for_each(|(folder, key)| self.dir(folder.clone(), key.clone()))
            .and_then(|()| self.claim_file(root, canonical_root, &path, key, stamp.clone()));
        match claimed {
            Ok(()) => Ok(()),
            Err(why) => Err(self.take_back(&path, stamp.as_ref(), new_dirs, why)),
        }
    }

    /// Lists a publication in memory only after its durable claim succeeds.
    fn claim_file(
        &mut self,
        root: &Path,
        canonical_root: &Path,
        path: &Path,
        key: PathBuf,
        stamp: Option<Stamp>,
    ) -> Result<(), String> {
        let Some(ledger) = self.ledger().cloned() else {
            return Ok(());
        };
        if self.has(&self.files, &key) {
            return Ok(());
        }
        self.note_root(root);
        ledger
            .lock_creating()
            .create_file(self.holder, key.clone(), canonical_root, stamp)?;
        // Listed only once the claim is on disk: a file the ledger does not know is not "owned".
        self.files.push(Owned {
            path: path.to_path_buf(),
            key,
            ledger,
            created: true,
        });
        Ok(())
    }

    /// Undoes one publication whose claims could not be written (see [`Created::published`]).
    fn take_back(
        &mut self,
        path: &Path,
        stamp: Option<&Stamp>,
        new_dirs: &[(PathBuf, PathBuf)],
        why: String,
    ) -> Unclaimed {
        let mut report = CleanupReport::default();
        remove_file(path, stamp, &mut report);
        let mut taken_back = Vec::new();
        if report.removed_files > 0 {
            taken_back.push(path.to_path_buf());
        }
        let mut kept = report.kept;
        let mut forget = Vec::new();
        for (folder, key) in new_dirs.iter().rev() {
            // A folder that was claimed stays this record's: a normal discard decides about it.
            if self.has(&self.dirs, key) {
                continue;
            }
            forget.push(folder.clone());
            match fs::remove_dir(folder) {
                Ok(()) => taken_back.push(folder.clone()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => kept.push(format!("`{}`: {error}", folder.display())),
            }
        }
        Unclaimed {
            why,
            taken_back,
            kept,
            forget,
        }
    }

    /// Relies on the file at `relative` if a pending result created it (the file was reused).
    /// Refuses (`claims_unavailable`) when the ledger cannot say, because then a discard of the
    /// other result could remove what this one relies on.
    pub(crate) fn join_file(
        &mut self,
        root: &Path,
        canonical_root: &Path,
        relative: &ResourcePath,
    ) -> Result<(), JobError> {
        let Some(ledger) = self.ledger().cloned() else {
            return Ok(());
        };
        let (path, key) = locate(root, canonical_root, relative);
        if self.has(&self.files, &key) {
            return Ok(());
        }
        let mut tracker = ledger.lock();
        let joined = tracker.join(self.holder, &key);
        if let Some(why) = tracker.fault() {
            return Err(claims_unavailable(why));
        }
        drop(tracker);
        if joined {
            self.note_root(root);
            self.files.push(Owned {
                path,
                key,
                ledger,
                created: false,
            });
        }
        Ok(())
    }

    /// Matches canonical ownership keys, independently of the caller spelling a path.
    fn has(&self, list: &[Owned], key: &Path) -> bool {
        list.iter().any(|owned| owned.key == key)
    }

    /// Retains each root once so removal can acquire all publication leases.
    fn note_root(&mut self, root: &Path) {
        if !self.roots.iter().any(|known| known == root) {
            self.roots.push(root.to_path_buf());
        }
    }

    /// Appends what `other` created and relies on after what this one has. Both become one
    /// holder: `other`'s entries are renamed to this record, **on disk first**.
    ///
    /// Nothing moves in memory until the ledger says the transfer is written. On `Err` both
    /// records are exactly as they were, so whoever owned a file before still does (if the write
    /// did reach the disk the entries are held by this record and `other` finds them shared when
    /// it leaves: a file may then outlive both, never be lost under an owner). Several ledgers
    /// are only possible for a record that mixes workspaces, which nothing does; if one of them
    /// fails after another succeeded, the same argument applies to the ones already moved.
    pub(crate) fn absorb(&mut self, other: &mut Created) -> Result<(), String> {
        for (ledger, keys) in by_ledger(other.files.iter().chain(&other.dirs)) {
            ledger.lock().transfer(
                other.holder,
                self.holder,
                keys.iter().map(|key| key.as_path()),
            )?;
        }
        if self.ledger.is_none() {
            self.ledger = other.ledger.take();
        }
        let files = std::mem::take(&mut other.files);
        let dirs = std::mem::take(&mut other.dirs);
        for root in std::mem::take(&mut other.roots) {
            self.note_root(&root);
        }
        merge(&mut self.files, files);
        merge(&mut self.dirs, dirs);
        Ok(())
    }

    /// The result was accepted: its entries are removed from the ledger, and only then is the
    /// record emptied. `Ok` means the claims are gone from the disk, so nothing (a discard, a
    /// recovery after this process ends) can take the files back. `Err` means that is not known:
    /// the record keeps everything, stays claimed by this process, and the caller must undo
    /// whatever it did to accept the result and discard or retry; discarding is safe in either
    /// case (it finds entries still there and removes, or finds none and keeps the files).
    pub(crate) fn accept(&mut self) -> Result<(), String> {
        self.accept_waiting(true)
    }

    /// [`Created::accept`] for a caller that must not block (the window's tick): a ledger lock
    /// still held after the tick's fixed budget (20 ms) is not waited for any longer. It answers
    /// `Err` like any ledger that cannot record the acceptance, and the record keeps everything.
    pub(crate) fn accept_nowait(&mut self) -> Result<(), String> {
        self.accept_waiting(false)
    }

    /// `wait` says whether the ledger lock may be waited for (bounded, see `ledger::LOCK_WAIT`).
    fn accept_waiting(&mut self, wait: bool) -> Result<(), String> {
        let mut failure = None;
        let mut accepted: Vec<PathBuf> = Vec::new();
        for (ledger, keys) in by_ledger(self.files.iter().chain(&self.dirs)) {
            let mut tracker = if wait {
                ledger.lock()
            } else {
                ledger.try_lock()
            };
            match tracker.accept(keys.iter().map(|key| key.as_path())) {
                Ok(()) => accepted.push(ledger.dir().to_path_buf()),
                Err(why) => {
                    failure.get_or_insert(why);
                }
            }
        }
        // What belongs to a ledger that did accept is the project's now; the rest stays listed.
        self.files
            .retain(|owned| !accepted.iter().any(|dir| dir == owned.ledger.dir()));
        self.dirs
            .retain(|owned| !accepted.iter().any(|dir| dir == owned.ledger.dir()));
        failure.map_or(Ok(()), Err)
    }

    /// Removes exactly what this record created and nobody else relies on: files newest first,
    /// then the folders deepest first. Never fails: what could not be removed is in the report.
    pub fn remove(mut self) -> CleanupReport {
        let holder = self.holder;
        let files = std::mem::take(&mut self.files);
        let dirs = std::mem::take(&mut self.dirs);
        let roots = std::mem::take(&mut self.roots);
        let mut report = CleanupReport::default();
        if files.is_empty() && dirs.is_empty() {
            return report;
        }
        let leases = take_leases(&roots, &mut report);
        for file in files.iter().rev() {
            // Decided under the root lease: whoever reuses a file claims it holding that same
            // lease, so a claim cannot slip in between this answer and the unlink below.
            // A root that stayed busy cannot be touched: the claim stays (as an orphan) so that
            // `recover` takes the file back later, instead of forgetting a file left behind.
            //
            // The last holder is *not* written out before the file is dealt with (F1): the entry
            // (and with it this process's lease) stays on disk through the SHA-256 and the
            // unlink, and is dropped only afterwards. A process killed in the middle of hashing a
            // large file therefore leaves an orphan claim that `recover` takes back, never a
            // file the ledger does not list.
            let verdict = if leases.blocked(&file.path) {
                file.ledger.lock().abandon(holder, &file.key)
            } else {
                file.ledger.lock().begin_leave(holder, &file.key)
            };
            match verdict {
                Leave::Untracked => {
                    if file.created {
                        report.shared.push(format!(
                            "`{}`: an accepted result owns it",
                            file.path.display()
                        ));
                    }
                }
                Leave::Shared => report.shared.push(format!(
                    "`{}`: another result still relies on it",
                    file.path.display()
                )),
                Leave::Unverifiable(why) => report.kept.push(format!(
                    "`{}`: the claims ledger cannot vouch for it ({why})",
                    file.path.display()
                )),
                Leave::Abandoned => report.kept.push(format!(
                    "`{}`: its root is busy; it stays claimed for recovery",
                    file.path.display()
                )),
                Leave::Last(item) => {
                    if let Item::File(stamp) = &item {
                        remove_file(&file.path, stamp.as_ref(), &mut report);
                    }
                    finish_leave(file, holder, &mut report);
                }
            }
        }
        // A folder below a root this call leases goes while the lease is still held: an
        // adoption that took the lease after ours has not looked at the folder yet, and one
        // that took it before is finished (its claims are in place), so nobody publishes into a
        // folder that is about to disappear.
        let inside: Vec<&Owned> = dirs
            .iter()
            .rev()
            .filter(|dir| {
                leases
                    .held
                    .iter()
                    .any(|lease| dir.key != lease.root() && dir.key.starts_with(lease.root()))
            })
            .collect();
        for dir in &inside {
            leave_dir(dir, holder, &mut report);
        }
        // The lock file lives in the root being cleaned: release before removing folders.
        drop(leases);
        for dir in dirs.iter().rev() {
            if !inside.iter().any(|done| done.key == dir.key) {
                leave_dir(dir, holder, &mut report);
            }
        }
        report
    }
}

/// Gives up a folder this record made or relied on, and removes it when it was the last holder
/// and it is empty. Done while the folder's entry is locked: another result creating or joining
/// the same folder waits, so it never finds a folder that is about to disappear. A folder with
/// claims still below it (an orphan, or another holder's file) stays claimed.
fn leave_dir(dir: &Owned, holder: HolderId, report: &mut CleanupReport) {
    let mut tracker = dir.ledger.lock();
    let verdict = if tracker.has_below(&dir.key) {
        tracker.abandon(holder, &dir.key)
    } else {
        tracker.begin_leave(holder, &dir.key)
    };
    match verdict {
        Leave::Untracked => {}
        Leave::Last(item) => {
            if item == Item::Dir {
                remove_dir(&dir.path, report);
            }
            drop_claim(&mut tracker, dir, holder, report);
        }
        Leave::Shared => report.shared.push(format!(
            "`{}`: another result still relies on it",
            dir.path.display()
        )),
        Leave::Abandoned => report.shared.push(format!(
            "`{}`: claims remain below it, so it stays claimed",
            dir.path.display()
        )),
        Leave::Unverifiable(why) => report.kept.push(format!(
            "`{}`: the claims ledger cannot vouch for it ({why})",
            dir.path.display()
        )),
    }
}

/// Drops the claim of `owned` after what it listed was dealt with (removed or deliberately
/// kept). When the ledger cannot record that, the claim stays on disk, held by this live process:
/// `recover` finds an entry whose file is gone (or still there and provably ours) once the process
/// ends, so it is reported but never a lost file.
fn drop_claim(tracker: &mut Tracker, owned: &Owned, holder: HolderId, report: &mut CleanupReport) {
    if let Leave::Unverifiable(why) = tracker.leave(holder, &owned.key) {
        report.kept.push(format!(
            "`{}`: it was handled, but the claims ledger could not record that ({why}); the claim stays for recovery",
            owned.path.display()
        ));
    }
}

/// [`drop_claim`] for a file, taking the ledger lock for just that step.
fn finish_leave(file: &Owned, holder: HolderId, report: &mut CleanupReport) {
    let mut tracker = file.ledger.lock();
    drop_claim(&mut tracker, file, holder, report);
}

/// A bare record that is dropped without [`Created::accept`] or [`Created::remove`] is treated as
/// accepted, **best effort**: it is the contract of a bare [`Adoption`](super::Adoption) that is
/// dropped (its files stay), kept for callers that do not care. A destructor cannot report, so
/// when the ledger cannot be written the entries stay claimed by this process and become orphans
/// when it ends: that is **not** an acceptance anything may rely on. Every flow that changes a
/// project (`import::apply`, `PendingAdoption::apply`) calls [`Created::accept`] explicitly,
/// as its last step, and undoes itself when that fails.
impl Drop for Created {
    fn drop(&mut self) {
        if self.files.is_empty() && self.dirs.is_empty() {
            return;
        }
        let _ = self.accept();
    }
}

/// `keys` of the entries grouped by the ledger they live in (one per workspace in practice).
fn by_ledger<'a>(owned: impl Iterator<Item = &'a Owned>) -> Vec<(Ledger, Vec<PathBuf>)> {
    let mut groups: Vec<(Ledger, Vec<PathBuf>)> = Vec::new();
    for item in owned {
        match groups
            .iter_mut()
            .find(|(ledger, _)| ledger.dir() == item.ledger.dir())
        {
            Some((_, keys)) => keys.push(item.key.clone()),
            None => groups.push((item.ledger.clone(), vec![item.key.clone()])),
        }
    }
    groups
}

/// A published file or folder the ledger could not claim, and what was done about it (see
/// [`Created::published`]).
#[derive(Debug, Default)]
pub(super) struct Unclaimed {
    /// Why the ledger could not be written (the first reason, when there were several).
    pub why: String,
    /// What was removed again: files and folders, as the caller spells them.
    pub taken_back: Vec<PathBuf>,
    /// What could not be removed, with the reason. Never claimed, never removed later.
    pub kept: Vec<String>,
    /// Folders that are not claimed by this record (removed or kept): a caller that tracks which
    /// folders it already recorded must forget them, so the next file records them again.
    pub forget: Vec<PathBuf>,
}

impl Unclaimed {
    /// Folds a later failure of the same operation into this one.
    pub(super) fn merge(&mut self, other: Unclaimed) {
        self.taken_back.extend(other.taken_back);
        self.kept.extend(other.kept);
        self.forget.extend(other.forget);
    }

    /// Whether something stays on disk that the ledger does not know about.
    pub(super) fn left_behind(&self) -> bool {
        !self.kept.is_empty()
    }

    /// `claims_unavailable`, said as it happened: after the copy, so not "nothing was changed".
    pub(super) fn into_error(self) -> JobError {
        let left = if self.kept.is_empty() {
            String::new()
        } else {
            format!("; left in place: {}", self.kept.join("; "))
        };
        JobError::new(
            "claims_unavailable",
            format!(
                "the claims ledger of this workspace stopped working after the adoption had started changing it ({}); the adoption was refused and the files it had just copied are taken back where possible{left}",
                self.why
            ),
        )
        .with_details(json!({
            "phase": "after_changes",
            "why": self.why,
            "takenBack": self.taken_back.iter().map(|path| path.display().to_string()).collect::<Vec<_>>(),
            "kept": self.kept,
        }))
    }
}

/// The ledger stopped working after this result started changing the workspace (it could not
/// record that this result relies on a file or folder another one made).
fn claims_unavailable(why: &str) -> JobError {
    JobError::new(
        "claims_unavailable",
        format!(
            "the claims ledger of this workspace stopped working while this result was being recorded ({why}); the adoption was refused and what it copied is taken back where the ledger allows"
        ),
    )
    .with_details(json!({ "phase": "after_changes", "why": why }))
}

/// The ledger could not record that the result was accepted. Not accepted: nothing was promised.
fn accept_unavailable(why: &str) -> JobError {
    JobError::new(
        "claims_unavailable",
        format!(
            "the claims ledger of this workspace could not record that the result was accepted ({why}); it was not accepted and what it copied stays claimed (a discard keeps whatever it cannot prove is its own)"
        ),
    )
    .with_details(json!({ "phase": "accept", "why": why }))
}

/// Adds what discarding the refused result did to a `claims_unavailable` error: what was removed
/// and, above all, what stays (it is never forgotten silently).
pub(super) fn with_cleanup(mut error: JobError, report: &CleanupReport) -> JobError {
    let mut details = match error.details.take() {
        Some(details @ Value::Object(_)) => details,
        _ => json!({}),
    };
    let mut cleanup = report.to_json();
    if let Some(previous) = details.get("cleanup").filter(|value| value.is_object()) {
        for key in ["removedFiles", "removedFolders", "busyRoots"] {
            cleanup[key] =
                json!(cleanup[key].as_u64().unwrap_or(0) + previous[key].as_u64().unwrap_or(0));
        }
        for key in ["kept", "shared"] {
            if let Some(items) = previous[key].as_array() {
                cleanup[key]
                    .as_array_mut()
                    .expect("cleanup array")
                    .extend(items.iter().cloned());
            }
        }
    }
    details["cleanup"] = cleanup;
    if !report.kept.is_empty() {
        error.message = format!(
            "{}; left in place: {}",
            error.message,
            report.kept.join("; ")
        );
    }
    error.with_details(details)
}

/// A refused acceptance as the project-facing error `claims_unavailable` (the code is kept, not
/// folded into `job_failed`), with what discarding the result did. `what` says what was not done.
pub(super) fn refused_accept(
    error: JobError,
    report: Option<&CleanupReport>,
    what: &str,
) -> CommandError {
    let error = match report {
        Some(report) => with_cleanup(error, report),
        None => error,
    };
    let mut refused = CommandError::new("claims_unavailable", format!("{}; {what}", error.message));
    if let Some(details) = error.details {
        refused = refused.details(details);
    }
    refused
}

/// Lists only publications made by this result, excluding resources it merely reused.
fn created(list: &[Owned]) -> impl Iterator<Item = &Path> {
    list.iter()
        .filter(|owned| owned.created)
        .map(|owned| owned.path.as_path())
}

/// Builds display and canonical keys from the same validated relative resource path.
fn locate(root: &Path, canonical_root: &Path, relative: &ResourcePath) -> (PathBuf, PathBuf) {
    let under = |base: &Path| {
        relative
            .segments()
            .fold(base.to_path_buf(), |path, segment| path.join(segment))
    };
    (under(root), under(canonical_root))
}

/// Appends `incoming` to `into`, once per key (a folder both records listed keeps its first
/// place, which keeps parents before children), and keeps "created" if either created it.
fn merge(into: &mut Vec<Owned>, incoming: Vec<Owned>) {
    for owned in incoming {
        match into.iter_mut().find(|known| known.key == owned.key) {
            Some(known) => known.created |= owned.created,
            None => into.push(owned),
        }
    }
}

/// Leases held while removing, and the roots that could not be taken.
#[derive(Default)]
pub(super) struct Leases {
    pub(super) held: Vec<RootLease>,
    busy: Vec<PathBuf>,
}

impl Leases {
    /// Prevents removal below any root whose publication lease could not be acquired.
    pub(super) fn blocked(&self, path: &Path) -> bool {
        self.busy.iter().any(|root| path.starts_with(root))
    }
}

/// Takes available root leases and records busy roots so their resources remain untouched.
pub(super) fn take_leases(roots: &[PathBuf], report: &mut CleanupReport) -> Leases {
    let mut leases = Leases::default();
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        match acquire(root) {
            Ok(lease) => leases.held.push(lease),
            Err(error) if error.code == "root_busy" => {
                report.busy_roots += 1;
                leases.busy.push(root.clone());
            }
            // Gone or unreadable: removal below reports what it cannot do.
            Err(_) => {}
        }
    }
    leases
}

/// Retries a busy publication lease within the bounded cleanup wait; other errors return at once.
fn acquire(root: &Path) -> Result<RootLease, tessel_editor_core::jobs::JobError> {
    let mut attempt = 0;
    loop {
        match RootLease::acquire(root) {
            Err(error) if error.code == "root_busy" && attempt + 1 < LEASE_ATTEMPTS => {
                attempt += 1;
                std::thread::sleep(LEASE_PAUSE);
            }
            other => return other,
        }
    }
}

/// Removes the file at `path` if, and only if, it is still what was published (BL097): same
/// identity, same content, and nothing wrote to it while it was being read.
pub(super) fn remove_file(path: &Path, stamp: Option<&Stamp>, report: &mut CleanupReport) {
    // A default token never cancels: this is the removal a destructor does, which cannot stop.
    let _ = remove_file_unless_cancelled(path, stamp, &CancelToken::default(), report);
}

/// [`remove_file`] whose content check stops **between blocks** when `token` is cancelled (the
/// hash reads in chunks and looks at the token before each one). A cancelled check removes
/// nothing: the file stays exactly as it was and `Err(cancelled)` is returned, with nothing
/// added to `report` for it, so the caller still lists it as not decided. Any other outcome is
/// `Ok` and is in the report.
pub(super) fn remove_file_unless_cancelled(
    path: &Path,
    stamp: Option<&Stamp>,
    token: &CancelToken,
    report: &mut CleanupReport,
) -> Result<(), JobError> {
    let keep = |report: &mut CleanupReport, why: &str| -> Result<(), JobError> {
        report.kept.push(format!("`{}`: {why}", path.display()));
        Ok(())
    };
    let before = match Observed::of(path) {
        // Already gone: a rollback inside the operation got there first.
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return keep(report, &error.to_string()),
        Ok(None) => {
            return keep(
                report,
                "it is not the file this operation published any more",
            );
        }
        Ok(Some(seen)) => seen,
    };
    // Without a stamp the file could not be identified when it was published; removing a file
    // that may not be ours is worse than keeping one that is.
    let Some(stamp) = stamp else {
        return keep(report, "it could not be identified when it was published");
    };
    if !stamp.is_the_file_of(&before) {
        return keep(
            report,
            "it is not the file this operation published any more",
        );
    }
    match hash_file(path, token, |_| {}) {
        Err(error) if error.is_cancelled() => return Err(error),
        Ok((hash, _)) if hash == stamp.sha256 => {}
        Ok(_) => {
            return keep(
                report,
                "its content changed after it was published (rewritten in place); the edit is kept",
            );
        }
        // Fail closed: what cannot be checked is not removed.
        Err(error) => {
            return keep(
                report,
                &format!(
                    "it could not be read to confirm it is unchanged: {}",
                    error.message
                ),
            );
        }
    }
    if Observed::of(path).ok().flatten().as_ref() != Some(&before) {
        return keep(report, "it changed while it was being checked");
    }
    match fs::remove_file(path) {
        Ok(()) => report.removed_files += 1,
        Err(error) => return keep(report, &error.to_string()),
    }
    Ok(())
}

/// Removes only an empty directory; missing is harmless and every other refusal is reported.
pub(super) fn remove_dir(dir: &Path, report: &mut CleanupReport) {
    match fs::remove_dir(dir) {
        Ok(()) => report.removed_dirs += 1,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => report.kept.push(format!("`{}`: {error}", dir.display())),
    }
}

/// Whether `open` (a project's workspace) is the folder `prepared` was copied into. Canonical
/// when both can be resolved, so a trailing slash or a link to the same folder is not "another".
pub(crate) fn same_folder(open: Option<&Path>, prepared: &Path) -> bool {
    let Some(open) = open else {
        return false;
    };
    if open == prepared {
        return true;
    }
    matches!(
        (fs::canonicalize(open), fs::canonicalize(prepared)),
        (Ok(left), Ok(right)) if left == right
    )
}

/// What a removal did. `kept` is never an error: it is everything that was deliberately or
/// unavoidably left, with the reason. `shared` is what was not this result's to remove.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CleanupReport {
    pub removed_files: usize,
    pub removed_dirs: usize,
    pub kept: Vec<String>,
    /// Files and folders left because another result relies on them, or an accepted one owns
    /// them (BL096). Not a leftover: they belong to someone else now.
    pub shared: Vec<String>,
    /// Roots another operation held for the whole wait.
    pub busy_roots: usize,
}

impl CleanupReport {
    /// Reports whether cleanup left any uncertain resources; shared resources are legitimate owners.
    pub fn is_clean(&self) -> bool {
        self.kept.is_empty()
    }

    /// Serializes removals and retained resources without treating shared ownership as a leftover.
    pub fn to_json(&self) -> Value {
        json!({
            "removedFiles": self.removed_files,
            "removedFolders": self.removed_dirs,
            "kept": self.kept,
            "shared": self.shared,
            "busyRoots": self.busy_roots,
        })
    }
}

/// Owns a [`Created`] until the result it belongs to is accepted. Dropping it removes what the
/// record created (and nobody else relies on); [`accept`](Provisional::accept) hands it over
/// to the project instead.
///
/// Not `Clone`: see the module docs.
#[derive(Debug, Default)]
pub struct Provisional {
    created: Created,
}

impl Provisional {
    /// Arms cleanup for a created record until explicit acceptance transfers it to the project.
    pub fn new(created: Created) -> Self {
        Self { created }
    }

    /// Borrows publication evidence without duplicating ownership or disarming cleanup.
    pub fn created(&self) -> &Created {
        &self.created
    }

    /// Takes over what `other` holds, writing the transfer to the ledger first (see
    /// [`Created::absorb`]). On `Err` both are untouched and `other` still owns its files, so
    /// the caller discards it (and says what that did) instead of losing track of them.
    pub(crate) fn absorb(&mut self, other: &mut Provisional) -> Result<(), JobError> {
        self.created
            .absorb(&mut other.created)
            .map_err(|why| claims_unavailable(&why))
    }

    /// Allows the import adapter to combine records while preserving the single cleanup owner.
    pub(crate) fn created_mut(&mut self) -> &mut Created {
        &mut self.created
    }

    /// The result was accepted: what it created now belongs to the project. **Fallible and
    /// explicit**: `Ok` only once the claims are removed from the ledger on disk. On `Err`
    /// (`claims_unavailable`, `details.phase = "accept"`) the result is *not* accepted: it is
    /// still owned here and still claimed, and the caller must reverse whatever it applied to
    /// the project and then [`discard`](Provisional::discard) it (which is safe whether or not
    /// the failed write reached the disk: a file whose claim is already gone is kept).
    ///
    /// Call it as the **last** step of applying the result, after everything that can fail.
    pub fn accept(&mut self) -> Result<(), JobError> {
        self.created
            .accept()
            .map_err(|why| accept_unavailable(&why))
    }

    /// [`Provisional::accept`] for the window's tick, which waits for the ledger lock only a fixed
    /// 20 ms (`ledger::TICK_WAIT`): one still held after that (another process, or another job of
    /// this one) refuses the acceptance (`claims_unavailable`, `details.phase = "accept"`,
    /// `details.why` says it was held and for how long it was waited for) instead of holding the
    /// tick up. Everything else is as in `accept`: nothing was promised and the result is still owned.
    pub fn accept_nowait(&mut self) -> Result<(), JobError> {
        self.created
            .accept_nowait()
            .map_err(|why| accept_unavailable(&why))
    }

    /// The result will not be used: removes what it created and says what it kept.
    pub fn discard(mut self) -> CleanupReport {
        std::mem::take(&mut self.created).remove()
    }
}

impl Drop for Provisional {
    fn drop(&mut self) {
        // Best effort by nature: a destructor cannot report. `discard` is the way to see it.
        let _ = std::mem::take(&mut self.created).remove();
    }
}
