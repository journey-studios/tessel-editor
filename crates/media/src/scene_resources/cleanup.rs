//! Ownership of what an adoption or an import wrote into the workspace (BL089, BL096, BL097), so a
//! result that is cancelled, rejected, late or discarded can take back exactly its own files and
//! nothing else.
//!
//! [`Created`] is a *record*: the files this operation published and the folders it made, in
//! creation order, plus what it merely **relies on** (a file another pending result copied and it
//! reused). [`Provisional`] is the *owner* of a record while the result is not yet accepted: it
//! removes the record when dropped unless the result was accepted ([`Provisional::release`]).
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
//! Accepting is releasing the record ([`Provisional::release`] and dropping the returned
//! [`Created`], or dropping a bare [`Created`]): its entries are deleted, which is what turns the
//! files into the project's. The registry belongs to one process; another process reusing the
//! same file is outside the guarantee (see `claims.rs`).
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
//! not block (a UI thread) should discard from a job (BL094).
//!
//! Removal first takes the same cooperative [`RootLease`] the publication used, so it cannot
//! interleave with another Tessel operation working in that root. A root that stays busy is
//! reported as `kept`, not forced.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde_json::{Value, json};
use tessel_editor_core::jobs::CancelToken;
use tessel_editor_core::scene_resources::ResourcePath;

use super::claims::{self, HolderId, Item, Leave, Stamp, Tracker};
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
    /// `false` for something the record only relies on (reused from another pending result).
    created: bool,
}

/// Files and folders one operation created, in creation order, and what it relies on that
/// another pending result created. Plain data plus a place in the claims registry: cloning it is
/// not possible on purpose (see the module docs); move it into a [`Provisional`] or call
/// [`Created::remove`]. Dropping a bare record means the result was accepted.
#[derive(Debug)]
pub struct Created {
    holder: HolderId,
    /// Roots whose lease removal takes first: where `files` were published or reused.
    roots: Vec<PathBuf>,
    files: Vec<Owned>,
    dirs: Vec<Owned>,
}

impl Default for Created {
    fn default() -> Self {
        Self {
            holder: claims::next_holder(),
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

    /// Records a folder this operation made, under the registry lock the caller already holds
    /// (so the folder and its entry appear together).
    pub(super) fn dir_with(&mut self, tracker: &mut Tracker, path: PathBuf, key: PathBuf) {
        if self.has(&self.dirs, &key) {
            return;
        }
        tracker.create_dir(self.holder, key.clone());
        self.dirs.push(Owned {
            path,
            key,
            created: true,
        });
    }

    /// Relies on a folder some pending result created, if one did. A folder nobody created is
    /// not recorded: nobody will remove it.
    pub(super) fn join_dir_with(&mut self, tracker: &mut Tracker, path: PathBuf, key: PathBuf) {
        if self.has(&self.dirs, &key) || !tracker.join(self.holder, &key) {
            return;
        }
        self.dirs.push(Owned {
            path,
            key,
            created: false,
        });
    }

    pub(super) fn dir(&mut self, path: PathBuf, key: PathBuf) {
        let mut tracker = claims::lock();
        self.dir_with(&mut tracker, path, key);
    }

    pub(super) fn join_dir(&mut self, path: PathBuf, key: PathBuf) {
        let mut tracker = claims::lock();
        self.join_dir_with(&mut tracker, path, key);
    }

    /// Records a file published at `relative` under `root` (`canonical_root` is the same folder
    /// resolved), with the SHA-256 it was verified to have.
    pub(crate) fn file(
        &mut self,
        root: &Path,
        canonical_root: &Path,
        relative: &ResourcePath,
        sha256: &str,
    ) {
        let (path, key) = locate(root, canonical_root, relative);
        if self.has(&self.files, &key) {
            return;
        }
        self.note_root(root);
        let stamp = Stamp::of(&path, sha256);
        claims::lock().create_file(self.holder, key.clone(), stamp);
        self.files.push(Owned {
            path,
            key,
            created: true,
        });
    }

    /// Relies on the file at `relative` if a pending result created it (the file was reused).
    pub(crate) fn join_file(
        &mut self,
        root: &Path,
        canonical_root: &Path,
        relative: &ResourcePath,
    ) {
        let (path, key) = locate(root, canonical_root, relative);
        if self.has(&self.files, &key) || !claims::lock().join(self.holder, &key) {
            return;
        }
        self.note_root(root);
        self.files.push(Owned {
            path,
            key,
            created: false,
        });
    }

    fn has(&self, list: &[Owned], key: &Path) -> bool {
        list.iter().any(|owned| owned.key == key)
    }

    fn note_root(&mut self, root: &Path) {
        if !self.roots.iter().any(|known| known == root) {
            self.roots.push(root.to_path_buf());
        }
    }

    /// Appends what `other` created and relies on after what this one has. Both become one
    /// holder: `other` is consumed and its entries are renamed to this record.
    pub(crate) fn absorb(&mut self, mut other: Created) {
        let files = std::mem::take(&mut other.files);
        let dirs = std::mem::take(&mut other.dirs);
        let roots = std::mem::take(&mut other.roots);
        claims::lock().transfer(
            other.holder,
            self.holder,
            files.iter().chain(&dirs).map(|owned| owned.key.as_path()),
        );
        for root in roots {
            self.note_root(&root);
        }
        merge(&mut self.files, files);
        merge(&mut self.dirs, dirs);
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
            let verdict = claims::lock().leave(holder, &file.key);
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
                Leave::Last(Item::Dir) => {}
                Leave::Last(Item::File(stamp)) => {
                    if leases.blocked(&file.path) {
                        report
                            .kept
                            .push(format!("`{}`: its root is busy", file.path.display()));
                        continue;
                    }
                    remove_file(&file.path, stamp.as_ref(), &mut report);
                }
            }
        }
        // The lock file lives in the root being cleaned: release before removing folders.
        drop(leases);
        // A folder is removed while its entry is locked: another result creating or joining the
        // same folder waits, so it never finds a folder that is about to disappear.
        let mut tracker = claims::lock();
        for dir in dirs.iter().rev() {
            match tracker.leave(holder, &dir.key) {
                Leave::Untracked | Leave::Last(Item::File(_)) => {}
                Leave::Shared => report.shared.push(format!(
                    "`{}`: another result still relies on it",
                    dir.path.display()
                )),
                Leave::Last(Item::Dir) => remove_dir(&dir.path, &mut report),
            }
        }
        report
    }
}

/// A bare record that is dropped is a result that was accepted: its files are the project's, so
/// the entries go and nobody removes them later. Removal consumes the lists first, which is why
/// it does not come through here.
impl Drop for Created {
    fn drop(&mut self) {
        if self.files.is_empty() && self.dirs.is_empty() {
            return;
        }
        claims::lock().accept(
            self.files
                .iter()
                .chain(&self.dirs)
                .map(|owned| owned.key.as_path()),
        );
    }
}

fn created(list: &[Owned]) -> impl Iterator<Item = &Path> {
    list.iter()
        .filter(|owned| owned.created)
        .map(|owned| owned.path.as_path())
}

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
struct Leases {
    held: Vec<RootLease>,
    busy: Vec<PathBuf>,
}

impl Leases {
    fn blocked(&self, path: &Path) -> bool {
        self.busy.iter().any(|root| path.starts_with(root))
    }
}

fn take_leases(roots: &[PathBuf], report: &mut CleanupReport) -> Leases {
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
fn remove_file(path: &Path, stamp: Option<&Stamp>, report: &mut CleanupReport) {
    let keep = |report: &mut CleanupReport, why: &str| {
        report.kept.push(format!("`{}`: {why}", path.display()));
    };
    let before = match Observed::of(path) {
        // Already gone: a rollback inside the operation got there first.
        Err(error) if error.kind() == io::ErrorKind::NotFound => return,
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
    match hash_file(path, &CancelToken::default(), |_| {}) {
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
        Err(error) => keep(report, &error.to_string()),
    }
}

fn remove_dir(dir: &Path, report: &mut CleanupReport) {
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
    pub fn is_clean(&self) -> bool {
        self.kept.is_empty()
    }

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
/// record created (and nobody else relies on); [`release`](Provisional::release) hands it over
/// to the project instead.
///
/// Not `Clone`: see the module docs.
#[derive(Debug, Default)]
pub struct Provisional {
    created: Created,
}

impl Provisional {
    pub fn new(created: Created) -> Self {
        Self { created }
    }

    pub fn created(&self) -> &Created {
        &self.created
    }

    pub(crate) fn absorb(&mut self, created: Created) {
        self.created.absorb(created);
    }

    pub(crate) fn created_mut(&mut self) -> &mut Created {
        &mut self.created
    }

    /// The result was accepted: what it created now belongs to the project and nothing is
    /// removed when this value drops. The returned record is that acceptance: **dropping it**
    /// makes the files nobody's to remove, for every other result that relied on them too.
    pub fn release(&mut self) -> Created {
        std::mem::take(&mut self.created)
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
