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
use super::cleanup::{CleanupReport, remove_dir, remove_file, take_leases};

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
    for (path, stamp, root) in &files {
        if let Err(error) = token.check() {
            cancelled = Some(error);
            break;
        }
        let leased_here = root.as_ref().is_some_and(|root| leased.contains(root));
        if !leased_here || leases.blocked(path) {
            // Busy root (another operation is working there) or no root recorded: not forced.
            recovery
                .report
                .kept
                .push(format!("`{}`: its root is busy or unknown", path.display()));
            continue;
        }
        // Under the root lease nothing can reuse the file, and it has no holder, so the content
        // check runs without the ledger lock.
        remove_file(path, stamp.as_ref(), &mut recovery.report);
        handled.push(path.clone());
    }
    {
        let mut tracker = ledger.lock();
        for path in &handled {
            // Dropped only if it is still an orphan: a holder that joined meanwhile keeps it.
            if tracker
                .entries()
                .any(|(key, entry)| key == path && entry.holders.is_empty())
            {
                tracker.forget(path);
            }
        }
    }
    // The lock file of a root lives inside it: release before removing folders.
    drop(leases);
    if let Some(error) = cancelled {
        return Err(error);
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
        tracker.forget(&dir);
    }
    Ok(recovery)
}

/// The claims of `workspace`, as `scene.resources.claims` reports them. Opening the ledger
/// reaps the holders whose lease is gone, so `reaped` says what that found.
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
