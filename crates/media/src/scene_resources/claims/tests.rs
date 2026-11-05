//! Unit tests of the claims ledger: ownership rules (BL096) and what makes them hold across
//! processes (BL111). The two-process proofs are in `tests/scene_claims_processes.rs`.

use std::fs::{self, File};
use std::path::{Path, PathBuf};

use super::*;
use crate::fsx::Staging;

fn workspace(name: &str) -> Staging {
    Staging::create(&std::env::temp_dir(), name).unwrap()
}

fn key(name: &str) -> PathBuf {
    PathBuf::from(format!("/te-claims-unit/{name}"))
}

fn state_path(dir: &Path) -> PathBuf {
    dir.join(".tessel-claims/claims.json")
}

fn root() -> PathBuf {
    PathBuf::from("/te-claims-unit")
}

#[test]
fn only_the_last_holder_to_leave_may_remove() {
    let dir = workspace("claims-last");
    let ledger = Ledger::of(dir.path());
    let (one, two, path) = (next_holder(), next_holder(), key("last-leaves"));
    let mut tracker = ledger.lock_creating();
    tracker.create_file(one, path.clone(), &root(), None);
    assert!(tracker.join(two, &path));
    assert_eq!(tracker.leave(one, &path), Leave::Shared);
    assert_eq!(tracker.leave(two, &path), Leave::Last(Item::File(None)));
    assert_eq!(tracker.leave(two, &path), Leave::Untracked, "gone for good");
}

#[test]
fn accepting_makes_the_entry_nobodys_to_remove() {
    let dir = workspace("claims-accepted");
    let ledger = Ledger::of(dir.path());
    let (one, two, path) = (next_holder(), next_holder(), key("accepted"));
    let mut tracker = ledger.lock_creating();
    tracker.create_dir(one, path.clone());
    tracker.join(two, &path);
    tracker.accept([path.as_path()].into_iter());
    assert_eq!(tracker.leave(one, &path), Leave::Untracked);
    assert_eq!(tracker.leave(two, &path), Leave::Untracked);
}

#[test]
fn joining_something_nobody_created_is_a_no_op() {
    let dir = workspace("claims-never");
    let ledger = Ledger::of(dir.path());
    let path = key("never-created");
    let mut tracker = ledger.lock();
    assert!(!tracker.join(next_holder(), &path));
    assert_eq!(tracker.leave(next_holder(), &path), Leave::Untracked);
    drop(tracker);
    assert!(
        !dir.path().join(".tessel-claims").exists(),
        "looking creates no ledger"
    );
}

#[test]
fn a_transferred_holder_keeps_the_entry_alive_under_its_new_name() {
    let dir = workspace("claims-transfer");
    let ledger = Ledger::of(dir.path());
    let (from, to, other, path) = (
        next_holder(),
        next_holder(),
        next_holder(),
        key("transferred"),
    );
    let mut tracker = ledger.lock_creating();
    tracker.create_file(from, path.clone(), &root(), None);
    tracker.join(other, &path);
    tracker.transfer(from, to, [path.as_path()].into_iter());
    assert_eq!(tracker.leave(other, &path), Leave::Shared);
    assert_eq!(tracker.leave(to, &path), Leave::Last(Item::File(None)));
}

#[test]
fn claims_outlive_a_tracker_and_a_ledger_with_nothing_in_it_disappears() {
    let dir = workspace("claims-persist");
    let ledger = Ledger::of(dir.path());
    let (one, path) = (next_holder(), key("persisted"));
    ledger
        .lock_creating()
        .create_file(one, path.clone(), &root(), None);
    assert!(state_path(dir.path()).is_file(), "written to disk");
    assert!(
        ledger
            .lock()
            .entries()
            .any(|(known, entry)| known == path && entry.holders.contains(&one)),
        "a new tracker (another process would be the same) sees it"
    );
    assert_eq!(
        ledger.lock().leave(one, &path),
        Leave::Last(Item::File(None))
    );
    assert!(
        !dir.path().join(".tessel-claims").exists(),
        "the workspace looks as if it never had a claim"
    );
}

/// Writes a ledger as another process would have left it.
fn write_ledger(dir: &Path, owner: u128, pid: u32, path: &Path) {
    let folder = dir.join(".tessel-claims");
    fs::create_dir_all(folder.join("live")).unwrap();
    File::create(folder.join("ledger.lock")).unwrap();
    let text = format!("{owner:032x}");
    fs::write(
        folder.join("claims.json"),
        serde_json::json!({
            "version": 1,
            "owners": { text.clone(): { "pid": pid, "since": 1 } },
            "entries": { path.to_str().unwrap(): {
                "kind": "file", "root": "/te-claims-unit", "stamp": null,
                "holders": [format!("{text}:1")],
            } },
        })
        .to_string(),
    )
    .unwrap();
}

#[test]
fn a_holder_whose_lease_is_gone_is_reaped_even_if_its_pid_is_alive() {
    let dir = workspace("claims-pid-reuse");
    let canonical = fs::canonicalize(dir.path()).unwrap();
    let ghost = 0xDEAD_u128;
    let path = key("pid-reuse");
    // The recorded PID is a live process (this one): it must not make the owner look alive.
    write_ledger(&canonical, ghost, std::process::id(), &path);
    let tracker = Ledger::of(dir.path()).lock();
    assert_eq!(tracker.reaped().len(), 1, "the ghost was reaped");
    assert_eq!(tracker.reaped()[0].pid, std::process::id());
    let entry = tracker
        .entries()
        .find(|(known, _)| *known == path)
        .map(|(_, entry)| entry.clone())
        .expect("its work stays as an orphan, to be decided about");
    assert!(entry.holders.is_empty());
}

#[test]
fn a_holder_whose_lease_is_held_is_alive_whatever_its_pid_says() {
    let dir = workspace("claims-lease-held");
    let canonical = fs::canonicalize(dir.path()).unwrap();
    let other = 0xBEEF_u128;
    let path = key("lease-held");
    // A PID that names nothing: only the lock says the owner lives.
    write_ledger(&canonical, other, 0, &path);
    let lease = canonical.join(format!(".tessel-claims/live/{other:032x}"));
    let held = File::create(&lease).unwrap();
    held.try_lock().unwrap();
    let tracker = Ledger::of(dir.path()).lock();
    assert!(tracker.reaped().is_empty(), "a held lease is a live owner");
    assert!(
        tracker
            .entries()
            .any(|(known, entry)| known == path && entry.holders.len() == 1)
    );
    drop(tracker);
    drop(held);
    let tracker = Ledger::of(dir.path()).lock();
    assert_eq!(tracker.reaped().len(), 1, "released, so gone");
}

#[test]
fn reusing_an_orphan_makes_the_new_holder_decide_about_it() {
    let dir = workspace("claims-orphan-join");
    let canonical = fs::canonicalize(dir.path()).unwrap();
    let path = key("orphan-join");
    write_ledger(&canonical, 0xDEAD, 0, &path);
    let ledger = Ledger::of(dir.path());
    let mine = next_holder();
    let mut tracker = ledger.lock();
    assert!(tracker.join(mine, &path), "an orphan can be joined");
    drop(tracker);
    assert_eq!(
        ledger.lock().leave(mine, &path),
        Leave::Last(Item::File(None)),
        "and with the dead one gone, leaving is the last"
    );
}

#[test]
fn an_unreadable_ledger_refuses_and_vouches_for_nothing() {
    let dir = workspace("claims-fault");
    let canonical = fs::canonicalize(dir.path()).unwrap();
    fs::create_dir_all(canonical.join(".tessel-claims")).unwrap();
    File::create(canonical.join(".tessel-claims/ledger.lock")).unwrap();
    fs::write(canonical.join(".tessel-claims/claims.json"), b"{ not json").unwrap();
    let ledger = Ledger::of(dir.path());
    let mut tracker = ledger.lock_creating();
    assert!(tracker.fault().is_some());
    let holder = next_holder();
    tracker.create_file(holder, key("faulty"), &root(), None);
    assert!(!tracker.join(holder, &key("faulty")));
    assert!(matches!(
        tracker.leave(holder, &key("faulty")),
        Leave::Unverifiable(_)
    ));
    drop(tracker);
    assert!(
        canonical.join(".tessel-claims/claims.json").is_file(),
        "the unreadable ledger is left for a person to look at"
    );
}

#[test]
fn a_ledger_of_a_newer_version_is_not_touched() {
    let dir = workspace("claims-version");
    let canonical = fs::canonicalize(dir.path()).unwrap();
    fs::create_dir_all(canonical.join(".tessel-claims")).unwrap();
    File::create(canonical.join(".tessel-claims/ledger.lock")).unwrap();
    fs::write(
        canonical.join(".tessel-claims/claims.json"),
        br#"{"version":2,"owners":{},"entries":{}}"#,
    )
    .unwrap();
    let tracker = Ledger::of(dir.path()).lock();
    assert!(tracker.fault().unwrap().contains("newer"));
}
