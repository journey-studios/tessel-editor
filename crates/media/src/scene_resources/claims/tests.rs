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
    tracker
        .create_file(one, path.clone(), &root(), None)
        .unwrap();
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
    tracker.create_dir(one, path.clone()).unwrap();
    tracker.join(two, &path);
    tracker.accept([path.as_path()].into_iter()).unwrap();
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
    tracker
        .create_file(from, path.clone(), &root(), None)
        .unwrap();
    tracker.join(other, &path);
    tracker
        .transfer(from, to, [path.as_path()].into_iter())
        .unwrap();
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
        .create_file(one, path.clone(), &root(), None)
        .unwrap();
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
fn the_last_holder_keeps_its_claim_until_the_removal_is_done() {
    let dir = workspace("claims-begin-leave");
    let canonical = fs::canonicalize(dir.path()).unwrap();
    let ledger = Ledger::of(dir.path());
    let (one, two, path) = (next_holder(), next_holder(), key("begin-leave"));
    let mut tracker = ledger.lock_creating();
    tracker
        .create_file(one, path.clone(), &root(), None)
        .unwrap();
    assert!(tracker.join(two, &path));
    // Not the last: it leaves at once, as `leave` does.
    assert_eq!(tracker.begin_leave(one, &path), Leave::Shared);
    // The last: told so, with the entry (and its owner) still on disk for whatever happens next.
    assert_eq!(
        tracker.begin_leave(two, &path),
        Leave::Last(Item::File(None))
    );
    drop(tracker);
    let on_disk = fs::read_to_string(state_path(&canonical)).unwrap();
    assert!(
        on_disk.contains("begin-leave"),
        "a process killed here leaves the claim: {on_disk}"
    );
    assert_eq!(
        ledger.lock().leave(two, &path),
        Leave::Last(Item::File(None))
    );
    assert!(
        !state_path(&canonical).exists(),
        "the claim goes once the removal is done"
    );
}

#[test]
fn leaving_for_good_is_refused_before_removing_when_the_ledger_cannot_be_written() {
    let dir = workspace("claims-begin-leave-unwritable");
    let canonical = fs::canonicalize(dir.path()).unwrap();
    let ledger = Ledger::of(dir.path());
    let (holder, bystander, path) = (next_holder(), next_holder(), key("begin-unwritable"));
    let mut tracker = ledger.lock_creating();
    tracker
        .create_file(holder, path.clone(), &root(), None)
        .unwrap();
    tracker
        .create_file(bystander, key("bystander"), &root(), None)
        .unwrap();
    drop(tracker);
    // A folder where the temporary file goes: the next write fails.
    fs::create_dir(canonical.join(".tessel-claims/claims.json.tmp")).unwrap();
    assert!(matches!(
        ledger.lock().begin_leave(holder, &path),
        Leave::Unverifiable(_)
    ));
    fs::remove_dir(canonical.join(".tessel-claims/claims.json.tmp")).unwrap();
    assert_eq!(
        ledger.lock().begin_leave(holder, &path),
        Leave::Last(Item::File(None)),
        "the claim is still there once the ledger works again"
    );
    ledger.lock().leave(holder, &path);
    ledger.lock().leave(bystander, &key("bystander"));
}

#[test]
fn waiting_for_a_lock_nobody_lets_go_of_is_bounded() {
    let dir = workspace("claims-lock-bound");
    let canonical = fs::canonicalize(dir.path()).unwrap();
    let ledger = Ledger::of(dir.path());
    let (holder, path) = (next_holder(), key("lock-bound"));
    ledger
        .lock_creating()
        .create_file(holder, path.clone(), &root(), None)
        .unwrap();
    // Another open file description holding the lock is what a stopped process does.
    let stuck = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(canonical.join(".tessel-claims/ledger.lock"))
        .unwrap();
    stuck.lock().unwrap();
    let started = std::time::Instant::now();
    let tracker = ledger.lock();
    let waited = started.elapsed();
    assert!(
        tracker
            .fault()
            .is_some_and(|why| why.contains("gave up after")),
        "{:?}",
        tracker.fault()
    );
    assert!(
        waited < std::time::Duration::from_secs(10),
        "it gave up instead of waiting forever: {waited:?}"
    );
    drop(tracker);
    stuck.unlock().unwrap();
    drop(stuck);
    assert_eq!(
        ledger.lock().leave(holder, &path),
        Leave::Last(Item::File(None)),
        "the claim was untouched by the refusal"
    );
}

/// The UI's ceiling for one tick (BL112.3). The tick's lock budget is far below it and the
/// elapsed time of a refused look is held to it (not to a loose second that ten times the
/// ceiling would still satisfy).
const TICK_CEILING: std::time::Duration = std::time::Duration::from_millis(100);

#[test]
fn the_tick_budget_is_fixed_and_well_below_the_ui_ceiling() {
    assert!(
        ledger::TICK_WAIT >= std::time::Duration::from_millis(10),
        "a budget of microseconds refuses results for contention inside this very process"
    );
    assert!(
        ledger::TICK_WAIT <= std::time::Duration::from_millis(25)
            && ledger::TICK_WAIT * 4 <= TICK_CEILING,
        "the tick cannot wait a good share of its ceiling: {:?}",
        ledger::TICK_WAIT
    );
}

#[test]
fn a_tick_that_finds_the_lock_held_for_a_moment_by_this_process_gets_it() {
    // N1: every `Tracker` opens its own descriptor and `flock` conflicts between descriptors of the
    // same process, so a discard, an adoption or a recovery of another slot holding the ledger for
    // a moment is a conflict too. Looking once refused a valid result for that.
    let dir = workspace("claims-lock-moment");
    let canonical = fs::canonicalize(dir.path()).unwrap();
    let ledger = Ledger::of(dir.path());
    let (holder, path) = (next_holder(), key("lock-moment"));
    ledger
        .lock_creating()
        .create_file(holder, path.clone(), &root(), None)
        .unwrap();
    let lock_path = canonical.join(".tessel-claims/ledger.lock");

    let ticking = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (held_tx, held_rx) = std::sync::mpsc::channel();
    let other_job = {
        let ticking = ticking.clone();
        std::thread::spawn(move || {
            let file = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(lock_path)
                .unwrap();
            file.lock().unwrap();
            held_tx.send(()).unwrap();
            // Held until the tick has started looking, and 2 ms more: a looks-once tick refuses.
            while !ticking.load(std::sync::atomic::Ordering::SeqCst) {
                std::thread::yield_now();
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
            file.unlock().unwrap();
        })
    };
    held_rx.recv().unwrap();
    ticking.store(true, std::sync::atomic::Ordering::SeqCst);
    let looked = std::time::Instant::now();
    let mut tracker = ledger.try_lock();
    eprintln!(
        "BUDGET {{\"measure\":\"tick-holder-2ms-same-process\",\"tryLockMs\":{:.3}}}",
        looked.elapsed().as_secs_f64() * 1000.0
    );
    assert!(
        tracker.fault().is_none(),
        "the lock was free again within the tick's budget: {:?}",
        tracker.fault()
    );
    assert_eq!(
        tracker.leave(holder, &path),
        Leave::Last(Item::File(None)),
        "the ledger was really opened and read"
    );
    drop(tracker);
    other_job.join().unwrap();
}

#[test]
fn a_tick_that_finds_the_lock_stuck_gives_up_inside_its_budget_with_a_neutral_reason() {
    let dir = workspace("claims-lock-stuck");
    let canonical = fs::canonicalize(dir.path()).unwrap();
    let ledger = Ledger::of(dir.path());
    let (holder, path) = (next_holder(), key("lock-stuck"));
    ledger
        .lock_creating()
        .create_file(holder, path.clone(), &root(), None)
        .unwrap();
    // Another open file description holding the lock is what a stopped process does (and what a
    // stuck job of this one does).
    let stuck = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(canonical.join(".tessel-claims/ledger.lock"))
        .unwrap();
    stuck.lock().unwrap();
    let started = std::time::Instant::now();
    let tracker = ledger.try_lock();
    let waited = started.elapsed();
    eprintln!(
        "BUDGET {{\"measure\":\"tick-holder-stuck-same-process\",\"tryLockMs\":{:.3}}}",
        waited.as_secs_f64() * 1000.0
    );
    let why = tracker.fault().expect("the lock stayed held: unavailable");
    assert!(
        why.contains("held right now") && why.contains("gave up after"),
        "{why}"
    );
    assert!(
        !why.contains("another process"),
        "a job of this process looks the same, so the reason does not blame one: {why}"
    );
    assert!(
        waited >= ledger::TICK_WAIT,
        "it waited its budget before giving up: {waited:?}"
    );
    assert!(
        waited < TICK_CEILING,
        "and no longer than the UI's ceiling: {waited:?}"
    );
    drop(tracker);
    stuck.unlock().unwrap();
    drop(stuck);
    // Free again: the same call gets the lock at once, and the claim was untouched.
    let mut tracker = ledger.try_lock();
    assert!(tracker.fault().is_none(), "{:?}", tracker.fault());
    assert_eq!(
        tracker.leave(holder, &path),
        Leave::Last(Item::File(None)),
        "the claim was untouched by the refusal"
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
    assert!(
        tracker
            .create_file(holder, key("faulty"), &root(), None)
            .is_err(),
        "a ledger that cannot be trusted does not claim: it says so"
    );
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

// --- a ledger that stops being writable after it was opened (BL111) ---------------------------

/// Makes the next write of the ledger fail: its temporary file is a folder, so the atomic
/// replace cannot start. Opening and reading are unaffected, which is what makes the case
/// "worked at first, broke later".
fn break_writes(dir: &Path) -> PathBuf {
    let blocker = dir.join(".tessel-claims/claims.json.tmp");
    fs::create_dir(&blocker).unwrap();
    blocker
}

fn on_disk(ledger: &Ledger, path: &Path) -> Option<Entry> {
    ledger
        .lock()
        .entries()
        .find(|(known, _)| *known == path)
        .map(|(_, entry)| entry.clone())
}

#[test]
fn a_claim_that_cannot_be_written_is_an_error_and_is_not_on_disk() {
    let dir = workspace("claims-write-fails");
    let canonical = fs::canonicalize(dir.path()).unwrap();
    let ledger = Ledger::of(dir.path());
    let (holder, written, lost) = (next_holder(), key("written"), key("lost"));
    ledger
        .lock_creating()
        .create_file(holder, written.clone(), &root(), None)
        .unwrap();
    let blocker = break_writes(&canonical);

    // The ledger opens fine (reads work, nothing changed yet): it is the *write* that breaks.
    let mut tracker = ledger.lock_creating();
    assert!(tracker.fault().is_none(), "opened and read without a fault");
    let error = tracker
        .create_file(holder, lost.clone(), &root(), None)
        .unwrap_err();
    assert!(error.contains("claims.json"), "{error}");
    assert!(tracker.fault().is_some(), "and the tracker knows it now");
    assert!(
        tracker.create_dir(holder, key("later")).is_err(),
        "nothing more is recorded once the ledger failed"
    );
    drop(tracker);

    fs::remove_dir(blocker).unwrap();
    assert!(on_disk(&ledger, &lost).is_none(), "never listed");
    assert!(
        on_disk(&ledger, &written).is_some(),
        "what was written stays"
    );
    assert_eq!(
        ledger.lock().leave(holder, &written),
        Leave::Last(Item::File(None))
    );
}

#[test]
fn leaving_is_not_reported_removable_when_the_ledger_could_not_record_it() {
    let dir = workspace("claims-leave-fails");
    let canonical = fs::canonicalize(dir.path()).unwrap();
    let ledger = Ledger::of(dir.path());
    let (holder, other, path) = (next_holder(), next_holder(), key("leave-fails"));
    // Another entry keeps the ledger from being emptied: emptying it removes the file instead of
    // replacing it, which does not need the temporary file that is blocked here.
    let mut tracker = ledger.lock_creating();
    tracker
        .create_file(next_holder(), key("bystander"), &root(), None)
        .unwrap();
    tracker
        .create_file(holder, path.clone(), &root(), None)
        .unwrap();
    drop(tracker);
    let blocker = break_writes(&canonical);

    // The last holder: removing the file would leave an entry (still on disk) for a file that
    // is gone, so it is told to keep it.
    assert!(matches!(
        ledger.lock().leave(holder, &path),
        Leave::Unverifiable(_)
    ));
    fs::remove_dir(&blocker).unwrap();
    let entry = on_disk(&ledger, &path).expect("the entry is still there for recovery");
    assert!(entry.holders.contains(&holder));

    // The same for a holder that is not the last, and for abandoning.
    assert!(ledger.lock().join(other, &path));
    break_writes(&canonical);
    assert!(matches!(
        ledger.lock().leave(other, &path),
        Leave::Unverifiable(_)
    ));
    assert!(matches!(
        ledger.lock().abandon(other, &path),
        Leave::Unverifiable(_)
    ));
}

#[cfg(unix)]
#[test]
fn a_path_the_ledger_cannot_write_down_is_refused_not_dropped() {
    use std::os::unix::ffi::OsStrExt;
    let dir = workspace("claims-not-utf8");
    let ledger = Ledger::of(dir.path());
    let odd = PathBuf::from(std::ffi::OsStr::from_bytes(b"/te-claims-unit/\xff\xfe"));
    let mut tracker = ledger.lock_creating();
    let error = tracker
        .create_file(next_holder(), odd, &root(), None)
        .unwrap_err();
    assert!(error.contains("UTF-8"), "{error}");
    assert!(tracker.fault().is_none(), "the ledger itself is fine");
}

// --- accepting and merging are written before they are believed (BL111.2) --------------------

/// Makes every write of the ledger fail, including removing the state file: the folder is
/// read-only. Opening the lock (an existing file) still works.
#[cfg(unix)]
fn read_only(dir: &Path, on: bool) {
    use std::os::unix::fs::PermissionsExt;
    let mode = if on { 0o555 } else { 0o755 };
    fs::set_permissions(dir.join(".tessel-claims"), fs::Permissions::from_mode(mode)).unwrap();
}

#[test]
fn an_accept_the_ledger_could_not_write_is_an_error_and_the_entry_stays_claimed() {
    let dir = workspace("claims-accept-fails");
    let canonical = fs::canonicalize(dir.path()).unwrap();
    let ledger = Ledger::of(dir.path());
    let (holder, bystander, path) = (next_holder(), next_holder(), key("accept-fails"));
    let mut tracker = ledger.lock_creating();
    tracker
        .create_file(bystander, key("bystander"), &root(), None)
        .unwrap();
    tracker
        .create_file(holder, path.clone(), &root(), None)
        .unwrap();
    drop(tracker);
    let blocker = break_writes(&canonical);

    let mut tracker = ledger.lock();
    let error = tracker
        .accept([path.as_path()].into_iter())
        .expect_err("an acceptance that was not written must not be reported");
    assert!(error.contains("claims.json"), "{error}");
    drop(tracker);
    fs::remove_dir(blocker).unwrap();

    let entry = on_disk(&ledger, &path).expect("still listed: not accepted");
    assert!(entry.holders.contains(&holder), "and still held");
    // A discard that follows keeps being right: it finds the entry and removes the file.
    assert_eq!(
        ledger.lock().leave(holder, &path),
        Leave::Last(Item::File(None))
    );
}

#[test]
fn a_faulty_ledger_refuses_to_accept_or_merge() {
    let dir = workspace("claims-accept-fault");
    let canonical = fs::canonicalize(dir.path()).unwrap();
    fs::create_dir_all(canonical.join(".tessel-claims")).unwrap();
    File::create(canonical.join(".tessel-claims/ledger.lock")).unwrap();
    fs::write(canonical.join(".tessel-claims/claims.json"), b"{ not json").unwrap();
    let mut tracker = Ledger::of(dir.path()).lock();
    let path = key("whatever");
    assert!(tracker.accept([path.as_path()].into_iter()).is_err());
    assert!(
        tracker
            .transfer(next_holder(), next_holder(), [path.as_path()].into_iter())
            .is_err()
    );
    assert!(tracker.forget(&path).is_err());
}

#[test]
fn a_transfer_the_ledger_could_not_write_leaves_the_owner_as_it_was() {
    let dir = workspace("claims-transfer-fails");
    let canonical = fs::canonicalize(dir.path()).unwrap();
    let ledger = Ledger::of(dir.path());
    let (from, to, path) = (next_holder(), next_holder(), key("transfer-fails"));
    let mut tracker = ledger.lock_creating();
    tracker
        .create_file(from, path.clone(), &root(), None)
        .unwrap();
    tracker
        .create_file(next_holder(), key("bystander"), &root(), None)
        .unwrap();
    drop(tracker);
    let blocker = break_writes(&canonical);

    let mut tracker = ledger.lock();
    assert!(
        tracker
            .transfer(from, to, [path.as_path()].into_iter())
            .is_err()
    );
    drop(tracker);
    fs::remove_dir(blocker).unwrap();
    let entry = on_disk(&ledger, &path).unwrap();
    assert!(entry.holders.contains(&from) && !entry.holders.contains(&to));
}

/// The lease says "this process is alive and holds claims". It must outlive a write that failed:
/// letting it go first would let another process reap this one while the disk still lists its
/// claims, and `recover` would remove files a live process (or an accepted result) owns.
#[cfg(unix)]
#[test]
fn a_failed_write_never_gives_up_the_lease_the_disk_still_depends_on() {
    let dir = workspace("claims-lease-kept");
    let canonical = fs::canonicalize(dir.path()).unwrap();
    let ledger = Ledger::of(dir.path());
    let (holder, path) = (next_holder(), key("lease-kept"));
    ledger
        .lock_creating()
        .create_file(holder, path.clone(), &root(), None)
        .unwrap();
    let leases = |canonical: &Path| {
        fs::read_dir(canonical.join(".tessel-claims/live"))
            .map(|names| names.count())
            .unwrap_or(0)
    };
    assert_eq!(leases(&canonical), 1);

    // The only claim of this process is accepted, which empties the ledger: the state file is
    // to be removed, and the folder does not allow it.
    read_only(&canonical, true);
    let mut tracker = ledger.lock();
    let result = tracker.accept([path.as_path()].into_iter());
    drop(tracker);
    read_only(&canonical, false);
    // (A process running as root ignores the mode: nothing to prove then.)
    if result.is_ok() {
        return;
    }
    assert_eq!(
        leases(&canonical),
        1,
        "the claims are still on disk, so the lease that vouches for their holder stays"
    );
    assert!(on_disk(&ledger, &path).is_some());
}

// --- a state without its lock is damage, not an empty ledger (BL111.2) --------------------------

#[test]
fn a_state_file_without_its_lock_is_a_fault_not_an_empty_ledger() {
    let dir = workspace("claims-dangling");
    let canonical = fs::canonicalize(dir.path()).unwrap();
    let ledger = Ledger::of(dir.path());
    let (holder, path) = (next_holder(), key("dangling"));
    ledger
        .lock_creating()
        .create_file(holder, path.clone(), &root(), None)
        .unwrap();
    fs::remove_file(canonical.join(".tessel-claims/ledger.lock")).unwrap();

    let tracker = ledger.lock();
    let fault = tracker.fault().expect("not an empty ledger");
    assert!(
        fault.contains("ledger.lock") && fault.contains("damaged"),
        "{fault}"
    );
    drop(tracker);
    assert!(matches!(
        ledger.lock().leave(holder, &path),
        Leave::Unverifiable(_)
    ));
    assert!(ledger.lock().accept([path.as_path()].into_iter()).is_err());
    assert!(
        state_path(&canonical).is_file(),
        "what the ledger lists is left for a person to look at"
    );
}

#[test]
fn a_folder_without_a_state_is_an_empty_ledger() {
    let dir = workspace("claims-empty-dir");
    let canonical = fs::canonicalize(dir.path()).unwrap();
    fs::create_dir_all(canonical.join(".tessel-claims")).unwrap();
    let tracker = Ledger::of(dir.path()).lock();
    assert!(tracker.fault().is_none());
    assert_eq!(tracker.entries().count(), 0);
}

/// The emptied ledger is removed (state, then lock, then folder) while others look at it and
/// claim again. A reader must never see that as damage.
#[test]
fn emptying_and_refilling_a_ledger_is_never_mistaken_for_damage() {
    let dir = workspace("claims-race");
    let ledger = Ledger::of(dir.path());
    let stop = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|scope| {
        for worker in 0..3 {
            let ledger = ledger.clone();
            let stop = &stop;
            scope.spawn(move || {
                for round in 0..150 {
                    let holder = next_holder();
                    let path = key(&format!("race-{worker}-{round}"));
                    let mut tracker = ledger.lock_creating();
                    tracker
                        .create_file(holder, path.clone(), &root(), None)
                        .unwrap();
                    drop(tracker);
                    assert!(matches!(ledger.lock().leave(holder, &path), Leave::Last(_)));
                }
                stop.store(true, std::sync::atomic::Ordering::SeqCst);
            });
        }
        let reader = ledger.clone();
        let stop = &stop;
        scope.spawn(move || {
            while !stop.load(std::sync::atomic::Ordering::SeqCst) {
                let tracker = reader.lock();
                assert!(tracker.fault().is_none(), "{:?}", tracker.fault());
            }
        });
    });
}

// --- the record: folders already held are not read from the ledger again ------------------------

#[test]
fn a_folder_the_record_already_holds_is_not_looked_up_in_the_ledger_again() {
    use std::sync::mpsc::channel;
    use std::time::Duration;

    let dir = workspace("claims-join-dir-held");
    let canonical = fs::canonicalize(dir.path()).unwrap();
    let folder = canonical.join("scenes");
    let other = canonical.join("elsewhere");
    let mut created = super::super::cleanup::Created::default();
    let ledger = created.bind(dir.path());
    {
        let mut tracker = ledger.lock_creating();
        created
            .dir_with(&mut tracker, folder.clone(), folder.clone())
            .unwrap();
    }
    // Hold the ledger lock: a lookup would wait for it for as long as it is held.
    let tracker = ledger.lock();
    let (done, answered) = channel();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let held = created.join_dir(folder.clone(), folder.clone());
            done.send(("held", held.is_ok())).unwrap();
            let fresh = created.join_dir(other.clone(), other.clone());
            done.send(("fresh", fresh.is_ok())).unwrap();
        });
        assert_eq!(
            answered.recv_timeout(Duration::from_secs(5)).ok(),
            Some(("held", true)),
            "a held folder is answered without waiting for the ledger"
        );
        assert!(
            answered.recv_timeout(Duration::from_millis(300)).is_err(),
            "a folder it does not hold does read the ledger, so it waits for the lock"
        );
        drop(tracker);
        assert_eq!(
            answered.recv_timeout(Duration::from_secs(5)).ok(),
            Some(("fresh", true))
        );
    });
}

// --- accepting in a large ledger appends a record instead of rewriting it (BL112.7) ------------

fn journal_path(canonical: &Path) -> PathBuf {
    canonical.join(".tessel-claims/claims.journal")
}

/// Fills the ledger with entries of `holder` until its state is past what an acceptance still
/// rewrites inline. Returns how many entries that took.
fn grow_past_inline(ledger: &Ledger, holder: HolderId, canonical: &Path) -> usize {
    let mut tracker = ledger.lock_creating();
    let mut count = 0;
    while fs::metadata(state_path(canonical)).map_or(0, |meta| meta.len()) <= ledger::INLINE_LIMIT {
        // A limit nobody can reach (a mutation of it) must fail the test, not fill the disk.
        assert!(count < 4096, "the ledger never got past the inline limit");
        tracker
            .create_file(holder, key(&format!("bulk-{count}")), &root(), None)
            .unwrap();
        count += 1;
    }
    count
}

/// A large ledger, plus entries of a result that is about to be accepted.
struct Large {
    _dir: Staging,
    canonical: PathBuf,
    ledger: Ledger,
    bulk: usize,
    result: HolderId,
}

fn large(name: &str, accepted: &[&str]) -> Large {
    let dir = workspace(name);
    let canonical = fs::canonicalize(dir.path()).unwrap();
    let ledger = Ledger::of(dir.path());
    let (bulk_holder, result) = (next_holder(), next_holder());
    let bulk = grow_past_inline(&ledger, bulk_holder, &canonical);
    let mut tracker = ledger.lock();
    for name in accepted {
        tracker
            .create_file(result, key(name), &root(), None)
            .unwrap();
    }
    drop(tracker);
    Large {
        _dir: dir,
        canonical,
        ledger,
        bulk,
        result,
    }
}

#[test]
fn a_large_ledger_is_accepted_by_one_appended_record_not_by_rewriting_it() {
    let world = large("claims-journal-append", &["accepted-one", "accepted-two"]);
    let (one, two) = (key("accepted-one"), key("accepted-two"));
    let before = fs::read(state_path(&world.canonical)).unwrap();
    assert!(before.len() as u64 > ledger::INLINE_LIMIT);

    world
        .ledger
        .accept([one.as_path(), two.as_path()].into_iter(), false)
        .unwrap();

    assert_eq!(
        fs::read(state_path(&world.canonical)).unwrap(),
        before,
        "the state file was neither read into a new document nor rewritten"
    );
    let journal = fs::read_to_string(journal_path(&world.canonical)).unwrap();
    assert_eq!(journal.lines().count(), 1, "one record: {journal}");
    assert!(journal.ends_with('\n'), "complete: {journal}");
    assert!(journal.contains("accepted-one") && journal.contains("accepted-two"));

    // The next reader sees them as gone, and only them.
    let tracker = world.ledger.lock();
    assert!(tracker.fault().is_none(), "{:?}", tracker.fault());
    assert_eq!(tracker.entries().count(), world.bulk);
    assert!(
        tracker
            .entries()
            .all(|(entry, _)| entry != one && entry != two)
    );
    drop(tracker);
    assert!(
        !journal_path(&world.canonical).exists(),
        "the reader folded the journal into the state before answering"
    );
    let state = fs::read_to_string(state_path(&world.canonical)).unwrap();
    assert!(!state.contains("accepted-one") && !state.contains("accepted-two"));
}

#[test]
fn a_journal_that_survived_its_compaction_changes_nothing_the_second_time() {
    // A crash between writing the state and removing the journal.
    let world = large("claims-journal-survives", &["survivor"]);
    let path = key("survivor");
    world
        .ledger
        .accept([path.as_path()].into_iter(), false)
        .unwrap();
    let journal = journal_path(&world.canonical);
    let record = fs::read(&journal).unwrap();
    assert_eq!(world.ledger.lock().entries().count(), world.bulk);
    assert!(!journal.exists());

    fs::write(&journal, &record).unwrap();
    let tracker = world.ledger.lock();
    assert!(tracker.fault().is_none(), "{:?}", tracker.fault());
    assert_eq!(tracker.entries().count(), world.bulk);
    drop(tracker);
    assert!(!journal.exists(), "and it is gone now");
}

#[test]
fn a_path_claimed_again_after_an_acceptance_is_not_taken_by_the_old_record() {
    let world = large("claims-journal-reclaim", &["reclaimed"]);
    let path = key("reclaimed");
    world
        .ledger
        .accept([path.as_path()].into_iter(), false)
        .unwrap();
    assert!(journal_path(&world.canonical).exists());

    // Whoever claims next opens the ledger, which folds the old record in before it writes.
    let again = next_holder();
    world
        .ledger
        .lock_creating()
        .create_file(again, path.clone(), &root(), None)
        .unwrap();
    assert!(!journal_path(&world.canonical).exists());
    let entry = on_disk(&world.ledger, &path).expect("claimed anew, the old record is gone");
    assert!(entry.holders.contains(&again), "{entry:?}");
    assert!(!entry.holders.contains(&world.result));
}

#[test]
fn a_record_that_never_finished_is_not_an_acceptance_and_the_next_writer_cuts_it_off() {
    let world = large("claims-journal-torn", &["torn-a", "torn-b", "torn-c"]);
    let (a, b, c) = (key("torn-a"), key("torn-b"), key("torn-c"));
    let journal = journal_path(&world.canonical);
    // One complete record, then a writer that died halfway through the next (no newline).
    let mut text = format!("{{\"accept\":[\"{}\"]}}\n", a.display());
    text.push_str(&format!("{{\"accept\":[\"{}", b.display()));
    fs::write(&journal, text).unwrap();

    world
        .ledger
        .accept([c.as_path()].into_iter(), false)
        .unwrap();
    let after = fs::read_to_string(&journal).unwrap();
    assert_eq!(after.lines().count(), 2, "the torn tail is gone: {after}");
    assert!(
        after.ends_with('\n') && !after.contains("torn-b"),
        "{after}"
    );

    let tracker = world.ledger.lock();
    assert!(tracker.fault().is_none(), "{:?}", tracker.fault());
    let left: Vec<&Path> = tracker
        .entries()
        .map(|(entry, _)| entry)
        .filter(|entry| entry.to_string_lossy().contains("/torn-"))
        .collect();
    assert_eq!(
        left,
        vec![b.as_path()],
        "only the unfinished one is left claimed"
    );
}

#[test]
fn a_torn_journal_alone_is_read_as_no_acceptance() {
    let world = large("claims-journal-torn-alone", &["torn-alone"]);
    let path = key("torn-alone");
    fs::write(
        journal_path(&world.canonical),
        format!("{{\"accept\":[\"{}\"", path.display()),
    )
    .unwrap();
    let tracker = world.ledger.lock();
    assert!(tracker.fault().is_none(), "{:?}", tracker.fault());
    assert!(
        tracker.entries().any(|(entry, _)| entry == path),
        "an acceptance that never completed did not happen"
    );
    drop(tracker);
    assert!(!journal_path(&world.canonical).exists());
}

#[test]
fn a_complete_line_that_does_not_parse_is_damage_and_the_ledger_refuses() {
    let world = large("claims-journal-damaged", &["damaged"]);
    let path = key("damaged");
    let journal = journal_path(&world.canonical);
    fs::write(&journal, b"{ not a record\n").unwrap();
    let state = fs::read(state_path(&world.canonical)).unwrap();

    let tracker = world.ledger.lock();
    let fault = tracker.fault().expect("damage is not an empty journal");
    assert!(fault.contains("claims journal"), "{fault}");
    drop(tracker);
    assert!(matches!(
        world.ledger.lock().leave(world.result, &path),
        Leave::Unverifiable(_)
    ));
    assert_eq!(
        fs::read(&journal).unwrap(),
        b"{ not a record\n",
        "left as it is for a person to look at"
    );
    assert_eq!(fs::read(state_path(&world.canonical)).unwrap(), state);
}

#[test]
fn an_acceptance_that_could_not_be_appended_is_an_error_and_the_entries_stay_claimed() {
    let world = large("claims-journal-fails", &["append-fails"]);
    let path = key("append-fails");
    // A folder where the journal file should be: the record cannot be written.
    let journal = journal_path(&world.canonical);
    fs::create_dir(&journal).unwrap();

    let error = world
        .ledger
        .accept([path.as_path()].into_iter(), false)
        .expect_err("an acceptance that was not written must not be reported");
    assert!(error.contains("claims.journal"), "{error}");
    fs::remove_dir(&journal).unwrap();

    let entry = on_disk(&world.ledger, &path).expect("still listed: not accepted");
    assert!(entry.holders.contains(&world.result), "and still held");
}

#[test]
fn the_tick_does_not_wait_for_a_stuck_holder_in_a_large_ledger_either() {
    let world = large("claims-journal-stuck", &["stuck"]);
    let path = key("stuck");
    let stuck = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(world.canonical.join(".tessel-claims/ledger.lock"))
        .unwrap();
    stuck.lock().unwrap();

    let started = std::time::Instant::now();
    let error = world
        .ledger
        .accept([path.as_path()].into_iter(), false)
        .expect_err("the lock stayed held: unavailable");
    let waited = started.elapsed();
    assert!(
        error.contains("held right now") && error.contains("gave up after"),
        "{error}"
    );
    assert!(
        waited >= ledger::TICK_WAIT,
        "it waited its budget: {waited:?}"
    );
    assert!(
        waited < TICK_CEILING,
        "and no longer than the ceiling: {waited:?}"
    );
    assert!(
        !journal_path(&world.canonical).exists(),
        "a refused acceptance writes nothing"
    );
    stuck.unlock().unwrap();
    drop(stuck);
    world
        .ledger
        .accept([path.as_path()].into_iter(), false)
        .unwrap();
}

#[test]
fn a_small_ledger_is_still_accepted_inline_and_an_emptied_one_disappears() {
    let dir = workspace("claims-inline-accept");
    let canonical = fs::canonicalize(dir.path()).unwrap();
    let ledger = Ledger::of(dir.path());
    let (holder, path) = (next_holder(), key("inline"));
    assert!(
        ledger.accept([path.as_path()].into_iter(), false).is_ok(),
        "nothing is claimed in a workspace without a ledger"
    );
    assert!(!canonical.join(".tessel-claims").exists());

    ledger
        .lock_creating()
        .create_file(holder, path.clone(), &root(), None)
        .unwrap();
    ledger.accept([path.as_path()].into_iter(), false).unwrap();
    assert!(
        !journal_path(&canonical).exists(),
        "small: rewritten inline"
    );
    assert!(
        !canonical.join(".tessel-claims").exists(),
        "the workspace looks as if it never had a claim"
    );
}

/// Several threads accepting (as the tick of several windows could) while others claim and
/// compact: every acceptance stays accepted and nothing is resurrected.
#[test]
fn concurrent_acceptances_and_compactions_never_lose_or_resurrect_an_entry() {
    let world = large("claims-journal-race", &[]);
    let ledger = &world.ledger;
    std::thread::scope(|scope| {
        for worker in 0..4 {
            scope.spawn(move || {
                for round in 0..40 {
                    let holder = next_holder();
                    let path = key(&format!("race-{worker}-{round}"));
                    ledger
                        .lock_creating()
                        .create_file(holder, path.clone(), &root(), None)
                        .unwrap();
                    // The lock may be held for a moment by another worker: a refused acceptance
                    // is retried (as a discard would then keep the entry), never skipped.
                    let accepted =
                        (0..50).any(|_| ledger.accept([path.as_path()].into_iter(), true).is_ok());
                    assert!(accepted, "{path:?} was never accepted");
                }
            });
        }
    });
    let tracker = ledger.lock();
    assert!(tracker.fault().is_none(), "{:?}", tracker.fault());
    assert_eq!(
        tracker.entries().count(),
        world.bulk,
        "every entry that was accepted is gone, and only those"
    );
}
