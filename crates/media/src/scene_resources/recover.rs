//! Recovery of what a result that no longer exists left claimed (BL111), and the read-only view of
//! the claims ledger of a workspace.
//!
//! A pending adoption or import lives in the memory of one process. When that process dies (a
//! crash, `kill -9`, power) the files it copied stay, claimed by a holder nobody can accept or
//! discard any more: an **orphan**. [`status`] says who holds what; [`recover`] takes the orphans
//! back, and only them:
//!
//! * a holder is gone when its owner's lease is gone (see `claims`), never because of a process
//!   ID, so a live editor with a recycled PID is not mistaken for a dead one;
//! * an entry that any live holder still relies on is not an orphan and is not touched;
//! * an accepted file has no entry at all, so it is not seen, whoever created it;
//! * a file is removed only if it is still the published one (same identity, same SHA-256, quiet
//!   while it is read), under the same root lease and the same check as any other cleanup
//!   (BL097); anything else is `kept` with the reason, and its entry is dropped;
//! * a folder is removed only when it is empty and no entry lies below it (`remove_dir`).
//!
//! Hashing is real work (it reads every orphan once): callers run it from a job, not from a UI
//! thread. A ledger that cannot be read or written refuses (`claims_unavailable`) and removes
//! nothing.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tessel_editor_core::jobs::{CancelToken, JobError};

use super::claims::{Item, Ledger, Stamp, process_owner};
use super::cleanup::{CleanupReport, remove_dir, remove_file_unless_cancelled, take_leases};

/// What a recovery found and did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Recovery {
    /// Owners whose lease was gone and whose claims were removed from the ledger.
    pub reaped_holders: usize,
    /// Entries left with no holder (files and folders) when it started.
    pub orphans: usize,
    pub report: CleanupReport,
}

impl Recovery {
    pub fn to_json(&self) -> Value {
        json!({
            "reapedHolders": self.reaped_holders,
            "orphans": self.orphans,
            "cleanup": self.report.to_json(),
        })
    }
}

fn unavailable(why: &str) -> JobError {
    JobError::new(
        "claims_unavailable",
        format!("the claims ledger of this workspace cannot be used; nothing was removed: {why}"),
    )
}

/// Reaps the holders whose lease is gone and removes the files and folders they left claimed,
/// when nothing alive relies on them and they are still what was published.
///
/// Errors: `claims_unavailable` (ledger unreadable or unwritable), `cancelled`. What was
/// removed before a cancellation stays removed and is no longer claimed.
pub fn recover(workspace: &Path, token: &CancelToken) -> Result<Recovery, JobError> {
    recover_with(workspace, token, &mut |_, _| {})
}

/// [`recover`] that tells `progress(done, total)` how far it is, counted in orphan **files**
/// (the work that reads bytes): once with `done == 0` when the files are known, then after each
/// one is decided. Cancellation is honoured **between files and between blocks of the hash** of
/// the file being checked: a cancel during a large file stops after the block being read, leaves
/// that file untouched and claimed, and the error carries the partial report plus
/// `undecidedFiles` (that file and the ones not reached). They stay orphans for the next run.
pub fn recover_with(
    workspace: &Path,
    token: &CancelToken,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<Recovery, JobError> {
    let ledger = Ledger::of(workspace);
    let mut recovery = Recovery::default();

    // Which roots hold orphan files: their leases are taken before anything is read again, so no
    // adoption can reuse (and so claim) an orphan between the decision and the unlink.
    let roots: Vec<PathBuf> = {
        let tracker = ledger.lock();
        if let Some(why) = tracker.fault() {
            return Err(unavailable(why));
        }
        recovery.reaped_holders = tracker.reaped().len();
        let mut roots: Vec<PathBuf> = tracker
            .entries()
            .filter(|(_, entry)| entry.holders.is_empty())
            .filter_map(|(_, entry)| entry.root.clone())
            .collect();
        roots.sort();
        roots.dedup();
        roots
    };
    let leases = take_leases(&roots, &mut recovery.report);
    let leased: Vec<PathBuf> = leases
        .held
        .iter()
        .map(|lease| lease.root().to_path_buf())
        .collect();

    // Decided again under the leases, files first.
    let files: Vec<(PathBuf, Option<Stamp>, Option<PathBuf>)> = {
        let tracker = ledger.lock();
        if let Some(why) = tracker.fault() {
            return Err(unavailable(why));
        }
        recovery.reaped_holders += tracker.reaped().len();
        recovery.orphans = tracker
            .entries()
            .filter(|(_, entry)| entry.holders.is_empty())
            .count();
        let mut files: Vec<_> = tracker
            .entries()
            .filter(|(_, entry)| entry.holders.is_empty())
            .filter_map(|(key, entry)| match &entry.item {
                Item::File(stamp) => Some((key.to_path_buf(), stamp.clone(), entry.root.clone())),
                Item::Dir => None,
            })
            .collect();
        // Newest first is not recorded; a stable order is enough (the report is sorted too).
        files.sort_by(|left, right| left.0.cmp(&right.0));
        files
    };
    let mut handled = Vec::new();
    let mut cancelled = None;
    // Orphan files whose content check was not reached or was interrupted: still claimed.
    let mut undecided = 0;
    progress(0, files.len());
    for (index, (path, stamp, root)) in files.iter().enumerate() {
        if let Err(error) = token.check() {
            cancelled = Some(error);
            undecided = files.len() - index;
            break;
        }
        let leased_here = root.as_ref().is_some_and(|root| leased.contains(root));
        if !leased_here || leases.blocked(path) {
            // Busy root (another operation is working there) or no root recorded: not forced.
            recovery
                .report
                .kept
                .push(format!("`{}`: its root is busy or unknown", path.display()));
            progress(index + 1, files.len());
            continue;
        }
        // Under the root lease nothing can reuse the file, and it has no holder, so the content
        // check runs without the ledger lock.
        // The hash of a large file looks at the token between blocks: a cancel during it stops
        // here with the file untouched and still claimed, so it is counted as not reached.
        if let Err(error) =
            remove_file_unless_cancelled(path, stamp.as_ref(), token, &mut recovery.report)
        {
            cancelled = Some(error);
            undecided = files.len() - index;
            break;
        }
        handled.push(path.clone());
        progress(index + 1, files.len());
    }
    {
        let mut tracker = ledger.lock();
        for path in &handled {
            // Dropped only if it is still an orphan: a holder that joined meanwhile keeps it.
            if tracker
                .entries()
                .any(|(key, entry)| key == path && entry.holders.is_empty())
                && let Err(why) = tracker.forget(path)
            {
                recovery.report.kept.push(format!(
                    "`{}`: the claims ledger could not record that it was handled ({why})",
                    path.display()
                ));
            }
        }
    }
    // The lock file of a root lives inside it: release before removing folders.
    drop(leases);
    if let Some(error) = cancelled {
        // What was done before the cancellation stays done: say how much.
        return Err(error.with_details(json!({
            "reapedHolders": recovery.reaped_holders,
            "orphans": recovery.orphans,
            "undecidedFiles": undecided,
            "cleanup": recovery.report.to_json(),
        })));
    }

    // Folders, deepest first, each while the ledger is locked (as a discard does), and only
    // when no claim lies below.
    let mut tracker = ledger.lock();
    if let Some(why) = tracker.fault() {
        return Err(unavailable(why));
    }
    let mut dirs: Vec<PathBuf> = tracker
        .entries()
        .filter(|(_, entry)| entry.holders.is_empty() && matches!(entry.item, Item::Dir))
        .map(|(key, _)| key.to_path_buf())
        .collect();
    dirs.sort_by_key(|dir| std::cmp::Reverse(dir.components().count()));
    for dir in dirs {
        if tracker.has_below(&dir) {
            continue;
        }
        remove_dir(&dir, &mut recovery.report);
        if let Err(why) = tracker.forget(&dir) {
            recovery.report.kept.push(format!(
                "`{}`: the claims ledger could not record that it was handled ({why})",
                dir.display()
            ));
        }
    }
    Ok(recovery)
}

/// The claims of `workspace`, as `scene.resources.claims` reports them. Not read-only: opening
/// the ledger reaps the holders whose lease is gone (so `reaped` says what that found), and a
/// ledger left with nothing in it is removed. It never removes a resource file or a folder of the
/// workspace, and never hashes one; `effects` repeats this in the answer.
pub fn status(workspace: &Path) -> Value {
    let ledger = Ledger::of(workspace);
    let tracker = ledger.lock();
    let me = process_owner();
    let short = |owner: u128| format!("{owner:032x}")[..16].to_string();
    let entries: Vec<Value> = tracker
        .entries()
        .map(|(key, entry)| {
            json!({
                "path": key.display().to_string(),
                "kind": match entry.item { Item::Dir => "dir", Item::File(_) => "file" },
                "orphan": entry.holders.is_empty(),
                "holders": entry.holders.iter()
                    .map(|holder| format!("{}:{}", short(holder.owner()), holder.seq()))
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    let orphans = entries
        .iter()
        .filter(|entry| entry["orphan"] == true)
        .count();
    json!({
        "action": "scene.resources.claims",
        "readOnly": false,
        "effects": "reaps the ledger entries of owners whose lease is gone; removes no resource file or folder",
        "workspace": ledger.dir().parent().map(|dir| dir.display().to_string()),
        "available": tracker.fault().is_none(),
        "fault": tracker.fault(),
        "owners": tracker.owners().map(|(token, owner)| json!({
            "owner": short(token),
            "pid": owner.pid,
            "since": owner.since,
            "thisProcess": token == me,
        })).collect::<Vec<_>>(),
        "reaped": tracker.reaped().iter().map(|owner| json!({
            "owner": short(owner.owner),
            "pid": owner.pid,
            "since": owner.since,
        })).collect::<Vec<_>>(),
        "orphans": orphans,
        "entries": entries,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use std::fs;
    use std::os::unix::fs::MetadataExt;

    use super::*;
    use crate::fsx::{Staging, hash_file};
    use crate::scene_resources::claims::next_holder;

    /// Files that are claimed with nobody holding them: what a crashed process leaves. The holder
    /// is this process, which gives the claim up without removing the file (`abandon`), the way a
    /// discard that finds its root busy does.
    fn orphans(workspace: &Path, files: &[(&str, &[u8])]) -> Vec<PathBuf> {
        let root = workspace.join("scenes/s/assets");
        fs::create_dir_all(&root).unwrap();
        let root = fs::canonicalize(&root).unwrap();
        let holder = next_holder();
        let mut paths = Vec::new();
        for (name, bytes) in files {
            let path = root.join(name);
            fs::write(&path, bytes).unwrap();
            let metadata = fs::metadata(&path).unwrap();
            let (sha256, _) = hash_file(&path, &CancelToken::default(), |_| {}).unwrap();
            let stamp = Stamp {
                len: metadata.len(),
                device: metadata.dev(),
                inode: metadata.ino(),
                sha256,
            };
            let ledger = Ledger::of(workspace);
            let mut tracker = ledger.lock_creating();
            tracker
                .create_file(holder, path.clone(), &root, Some(stamp))
                .unwrap();
            tracker.abandon(holder, &path);
            paths.push(path);
        }
        paths
    }

    fn claimed(workspace: &Path) -> usize {
        Ledger::of(workspace).lock().entries().count()
    }

    #[test]
    fn progress_counts_orphan_files_and_ends_with_the_ledger_gone() {
        let dir = Staging::create(&std::env::temp_dir(), "recover-progress").unwrap();
        let paths = orphans(dir.path(), &[("a", b"one"), ("b", b"two"), ("c", b"three")]);
        let mut seen = Vec::new();
        let recovery = recover_with(dir.path(), &CancelToken::default(), &mut |done, total| {
            seen.push((done, total));
        })
        .unwrap();
        assert_eq!(seen, [(0, 3), (1, 3), (2, 3), (3, 3)]);
        assert_eq!(recovery.report.removed_files, 3, "{recovery:?}");
        assert!(paths.iter().all(|path| !path.exists()));
        assert_eq!(claimed(dir.path()), 0);
    }

    #[test]
    fn cancelling_between_files_keeps_the_rest_claimed_and_reports_what_was_done() {
        let dir = Staging::create(&std::env::temp_dir(), "recover-cancel").unwrap();
        let paths = orphans(dir.path(), &[("a", b"one"), ("b", b"two"), ("c", b"three")]);
        let token = CancelToken::default();
        let error = recover_with(dir.path(), &token, &mut |done, _| {
            if done == 1 {
                token.cancel();
            }
        })
        .unwrap_err();
        assert!(error.is_cancelled(), "{error:?}");
        let details = error.details.expect("the partial report travels with it");
        assert_eq!(details["cleanup"]["removedFiles"], 1, "{details}");
        // Sorted by path: `a` went, `b` and `c` stay, still claimed, so a later run finishes.
        assert!(!paths[0].exists() && paths[1].exists() && paths[2].exists());
        assert_eq!(claimed(dir.path()), 2);
        let rest = recover(dir.path(), &CancelToken::default()).unwrap();
        assert_eq!(rest.report.removed_files, 2, "{rest:?}");
        assert_eq!(claimed(dir.path()), 0);
    }

    /// The hash of a large orphan is the only slow step of a recovery. A cancel that arrives while
    /// it runs must stop it between blocks, not wait for the whole file (a 2 GiB sparse file takes
    /// seconds to hash; the cancel comes after a few milliseconds).
    #[test]
    fn cancelling_during_the_hash_of_a_large_file_stops_it_and_keeps_the_file_claimed() {
        let dir = Staging::create(&std::env::temp_dir(), "recover-large").unwrap();
        let root = dir.path().join("scenes/s/assets");
        fs::create_dir_all(&root).unwrap();
        let root = fs::canonicalize(&root).unwrap();
        let path = root.join("big.bin");
        let file = fs::File::create(&path).unwrap();
        file.set_len(2 << 30).unwrap();
        drop(file);
        let metadata = fs::metadata(&path).unwrap();
        // A hash that cannot match: the file would be kept if the check ever finished.
        let stamp = Stamp {
            len: metadata.len(),
            device: metadata.dev(),
            inode: metadata.ino(),
            sha256: "0".repeat(64),
        };
        let holder = next_holder();
        let ledger = Ledger::of(dir.path());
        {
            let mut tracker = ledger.lock_creating();
            tracker
                .create_file(holder, path.clone(), &root, Some(stamp))
                .unwrap();
            tracker.abandon(holder, &path);
        }

        let token = CancelToken::default();
        let started = std::time::Instant::now();
        let error = std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(40));
                token.cancel();
            });
            recover_with(dir.path(), &token, &mut |_, _| {}).unwrap_err()
        });
        assert!(error.is_cancelled(), "{error:?}");
        let details = error.details.expect("the partial report travels with it");
        assert_eq!(details["undecidedFiles"], 1, "{details}");
        assert_eq!(details["cleanup"]["removedFiles"], 0, "{details}");
        assert_eq!(
            details["cleanup"]["kept"],
            json!([]),
            "decided nothing: {details}"
        );
        assert!(path.exists(), "untouched");
        assert_eq!(claimed(dir.path()), 1, "still claimed for the next run");
        eprintln!("cancelled the large hash after {:?}", started.elapsed());
    }

    #[test]
    fn a_ledger_that_cannot_be_read_removes_nothing_and_says_so() {
        let dir = Staging::create(&std::env::temp_dir(), "recover-unreadable").unwrap();
        let paths = orphans(dir.path(), &[("a", b"one")]);
        fs::write(dir.path().join(".tessel-claims/claims.json"), b"not json").unwrap();
        let error = recover(dir.path(), &CancelToken::default()).unwrap_err();
        assert_eq!(error.code, "claims_unavailable");
        assert!(paths[0].exists());
        let status = status(dir.path());
        assert_eq!(status["available"], false, "{status}");
        assert_eq!(status["readOnly"], false);
    }
}
