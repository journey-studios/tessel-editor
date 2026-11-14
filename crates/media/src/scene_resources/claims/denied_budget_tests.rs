//! Real Windows access denials of the ledger: they must not renew the tick's lock-wait budget
//! (BL167.6.1), and a denied creation of the ledger folder is retried inside that budget instead of
//! failing the caller at once (BL167.6.2).

use std::cell::Cell;
use std::fs::{self, OpenOptions};
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::time::{Duration, Instant};

use super::ledger::{CREATE_FOLDER, DENIED_RETRIES, TICK_WAIT};
use super::{Leave, Ledger, next_holder};
use crate::fsx::Staging;

const TICK_CEILING: Duration = Duration::from_millis(100);

fn timed<T>(operation: impl FnOnce() -> T) -> (T, Duration) {
    let started = Instant::now();
    let result = operation();
    (result, started.elapsed())
}

fn denied_open_keeps_claims_and_respects_the_tick_ceiling(directory: bool) {
    let workspace = Staging::create(&std::env::temp_dir(), "claims-denied-budget").unwrap();
    let canonical = fs::canonicalize(workspace.path()).unwrap();
    let ledger = Ledger::of(workspace.path());
    let holder = next_holder();
    let pending = canonical.join("pending");
    ledger
        .lock_creating()
        .create_file(holder, pending.clone(), &canonical, None)
        .unwrap();
    let state = canonical.join(".tessel-claims/claims.json");
    let before = fs::read(&state).unwrap();
    let lock = canonical.join(".tessel-claims/ledger.lock");
    let permissions = fs::metadata(&lock).unwrap().permissions();
    if directory {
        fs::remove_file(&lock).unwrap();
        fs::create_dir(&lock).unwrap();
    } else {
        let mut read_only = permissions.clone();
        read_only.set_readonly(true);
        fs::set_permissions(&lock, read_only).unwrap();
    }

    let denied = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock)
        .err()
        .map(|error| error.kind());
    let (zero, zero_time) = timed(|| ledger.lock_for(Duration::ZERO).fault().map(str::to_owned));
    let (tick, tick_time) = timed(|| ledger.try_lock().fault().map(str::to_owned));
    let (accept, accept_time) = timed(|| ledger.accept([pending.as_path()].into_iter(), false));

    // Restore before any assertion, including an elapsed-time failure, so the staging directory
    // can be removed on Windows and the ledger can prove its claim was kept.
    if directory {
        fs::remove_dir(&lock).unwrap();
        fs::write(&lock, []).unwrap();
    } else {
        fs::set_permissions(&lock, permissions).unwrap();
    }

    assert_eq!(denied, Some(std::io::ErrorKind::PermissionDenied));
    for (operation, fault, elapsed) in [
        ("lock_for(ZERO)", zero, zero_time),
        ("try_lock", tick, tick_time),
        ("accept(wait=false)", accept.err(), accept_time),
    ] {
        eprintln!("DENIED_BUDGET directory={directory} operation={operation} elapsed={elapsed:?}");
        assert!(
            fault.is_some_and(|why| why.contains("open `")),
            "{operation}"
        );
        assert!(
            elapsed < TICK_CEILING,
            "{operation} exceeded the tick ceiling: {elapsed:?}"
        );
    }
    assert_eq!(
        fs::read(state).unwrap(),
        before,
        "refusal changed the claims"
    );
    assert_eq!(
        ledger.lock().leave(holder, &pending),
        Leave::Last(super::Item::File(None)),
        "the pending result still owns its claim after all three refusals"
    );
}

#[test]
fn a_directory_at_the_lock_path_respects_zero_tick_and_accept_budgets() {
    denied_open_keeps_claims_and_respects_the_tick_ceiling(true);
}

#[test]
fn a_read_only_lock_respects_zero_tick_and_accept_budgets() {
    denied_open_keeps_claims_and_respects_the_tick_ceiling(false);
}

// --- creating the ledger folder is denied (BL167.6.2) --------------------------------------
//
// Two kinds of evidence, and neither proves what NTFS does in the race (that was only observed:
// `tests.rs` churn, the report). A real `create_dir` refusal, from a local deny rule on the folder
// of the test (no global ACL, no unsafe, lifted on drop), proves how a denial that stays is
// handled. A refusal **injected** at the call (`CREATE_FOLDER`, same error: os error 5) proves the
// control of the retries (go-ahead after a few denials, ZERO, the cap) with no clock to race.

/// Denies creating sub-folders in a folder to everyone (`(AD)`, add subdirectory) while it lives.
struct NoNewFolders(PathBuf);

impl NoNewFolders {
    fn deny(folder: &Path) -> Self {
        icacls(folder, ["/deny", "*S-1-1-0:(AD)"]);
        Self(folder.to_path_buf())
    }

    fn lift(&self) {
        icacls(&self.0, ["/remove:d", "*S-1-1-0"]);
    }
}

impl Drop for NoNewFolders {
    fn drop(&mut self) {
        self.lift();
    }
}

fn icacls<const N: usize>(folder: &Path, arguments: [&str; N]) {
    let output = Command::new("icacls")
        .arg(folder)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "icacls {arguments:?}: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

/// What the production code must treat as "denied", observed without it.
fn creating_is_denied(folder: &Path) {
    let error = fs::create_dir(folder).expect_err("creating is denied");
    assert_eq!(error.kind(), ErrorKind::PermissionDenied, "{error}");
    assert_eq!(error.raw_os_error(), Some(5), "{error}");
}

/// Makes this thread's folder creation answer "access denied" (os error 5) `refuse` times and
/// then do the real thing; counts the calls. Undone when dropped.
struct Refusing(Rc<Cell<usize>>);

impl Refusing {
    fn first(refuse: usize) -> Self {
        let calls = Rc::new(Cell::new(0));
        let seen = Rc::clone(&calls);
        CREATE_FOLDER.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |folder| {
                seen.set(seen.get() + 1);
                if seen.get() <= refuse {
                    Err(io::Error::from_raw_os_error(5))
                } else {
                    fs::create_dir(folder)
                }
            }));
        });
        Self(calls)
    }

    fn calls(&self) -> usize {
        self.0.get()
    }
}

impl Drop for Refusing {
    fn drop(&mut self) {
        CREATE_FOLDER.with(|hook| *hook.borrow_mut() = None);
    }
}

fn claims_work_and_the_ledger_goes_away(ledger: &Ledger, canonical: &Path) {
    let (holder, claim) = (next_holder(), canonical.join("claimed"));
    let mut tracker = ledger.lock_creating();
    assert!(tracker.fault().is_none(), "{:?}", tracker.fault());
    tracker
        .create_file(holder, claim.clone(), canonical, None)
        .unwrap();
    drop(tracker);
    assert!(
        canonical.join(".tessel-claims/claims.json").is_file(),
        "the claim is on disk"
    );
    assert_eq!(
        ledger.lock().leave(holder, &claim),
        Leave::Last(super::Item::File(None))
    );
    assert!(!canonical.join(".tessel-claims").exists());
}

fn staged(name: &str) -> (Staging, PathBuf) {
    let workspace = Staging::create(&std::env::temp_dir(), name).unwrap();
    let canonical = fs::canonicalize(workspace.path()).unwrap();
    (workspace, canonical)
}

/// The failure of the race: creating the ledger folder answers "access denied" a few times and
/// then stops. The caller gets its lock, claims and leaves as on any other ledger; before the
/// retry the first denial ended the call with `os error 5`. Injected refusals (see above).
#[test]
fn a_denied_create_that_goes_away_is_retried_to_a_lock() {
    let (_workspace, canonical) = staged("claims-create-away");
    let ledger = Ledger::of(&canonical);
    let refusing = Refusing::first(5);
    let tracker = ledger.lock_creating_for(Duration::from_secs(5));
    assert!(tracker.fault().is_none(), "{:?}", tracker.fault());
    assert_eq!(refusing.calls(), 6, "five refusals, then the create");
    drop(tracker);
    drop(refusing);
    claims_work_and_the_ledger_goes_away(&ledger, &canonical);
}

/// A caller that does not wait (`Duration::ZERO`) is refused by the first denial, with the
/// denial as its message and nothing created. Injected refusals.
#[test]
fn a_denied_create_is_not_retried_by_a_caller_that_does_not_wait() {
    let (_workspace, canonical) = staged("claims-create-zero");
    let ledger = Ledger::of(&canonical);
    let refusing = Refusing::first(usize::MAX);
    let tracker = ledger.lock_creating_for(Duration::ZERO);
    let why = tracker.fault().expect("refused").to_owned();
    assert!(
        why.contains(".tessel-claims") && !why.contains("open `") && why.contains("os error 5"),
        "{why}"
    );
    assert_eq!(refusing.calls(), 1, "no retry once the budget is spent");
    drop(tracker);
    drop(refusing);
    assert!(
        !canonical.join(".tessel-claims").exists(),
        "nothing was created"
    );
    claims_work_and_the_ledger_goes_away(&ledger, &canonical);
}

/// A denial that never goes away is believed after the cap on retries, whatever the budget:
/// not renewed, not endless. Injected refusals.
#[test]
fn a_denied_create_that_stays_is_given_up_at_the_retry_cap() {
    let (_workspace, canonical) = staged("claims-create-cap");
    let ledger = Ledger::of(&canonical);
    let refusing = Refusing::first(usize::MAX);
    let tracker = ledger.lock_creating_for(Duration::from_secs(5));
    let why = tracker.fault().expect("refused").to_owned();
    assert!(why.contains("os error 5"), "{why}");
    assert_eq!(
        refusing.calls(),
        DENIED_RETRIES as usize + 1,
        "the retries, then the denial is reported"
    );
}

/// A real denial that stays (no injection): ZERO and the tick are refused inside their budget, the
/// denial is the message, nothing is created, and the ledger works once the rule is lifted.
#[test]
fn a_real_denied_create_respects_the_zero_and_tick_budgets() {
    let (workspace, canonical) = staged("claims-create-real");
    let denial = NoNewFolders::deny(workspace.path());
    creating_is_denied(&canonical.join(".tessel-claims"));
    let ledger = Ledger::of(&canonical);

    let (zero, zero_time) = timed(|| {
        ledger
            .lock_creating_for(Duration::ZERO)
            .fault()
            .map(str::to_owned)
    });
    let (tick, tick_time) = timed(|| {
        ledger
            .lock_creating_for(TICK_WAIT)
            .fault()
            .map(str::to_owned)
    });
    assert!(
        !canonical.join(".tessel-claims").exists(),
        "nothing was created"
    );
    denial.lift();

    for (operation, fault, elapsed) in [
        ("lock_creating_for(ZERO)", zero, zero_time),
        ("lock_creating_for(TICK_WAIT)", tick, tick_time),
    ] {
        eprintln!("DENIED_CREATE operation={operation} elapsed={elapsed:?}");
        let why = fault.unwrap_or_else(|| panic!("{operation} was not refused"));
        assert!(
            why.contains(".tessel-claims") && !why.contains("open `") && why.contains("os error 5"),
            "{operation}: {why}"
        );
        assert!(elapsed < TICK_CEILING, "{operation} took {elapsed:?}");
    }
    claims_work_and_the_ledger_goes_away(&ledger, &canonical);
}
