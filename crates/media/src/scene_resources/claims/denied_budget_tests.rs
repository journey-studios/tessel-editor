//! Real Windows access denials must not renew the tick's lock-wait budget (BL167.6.1).

use std::fs::{self, OpenOptions};
use std::time::{Duration, Instant};

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
