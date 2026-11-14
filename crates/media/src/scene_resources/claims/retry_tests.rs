//! BL167.6.5: transient create/open failures must yield CPU without renewing the lock budget.
//! These tests replace sleep, so they judge retry control without wall-clock thresholds.

use std::cell::{Cell, RefCell};
use std::fs;
use std::io;
use std::rc::Rc;
use std::time::Duration;

use super::Ledger;
use super::ledger::CREATE_FOLDER;
use super::retry_wait::SLEEP;
use crate::fsx::Staging;

struct Hooks;

impl Drop for Hooks {
    fn drop(&mut self) {
        CREATE_FOLDER.with(|hook| *hook.borrow_mut() = None);
        SLEEP.with(|hook| *hook.borrow_mut() = None);
    }
}

fn transient_missing(create: bool, wait: Duration) {
    let workspace = Staging::create(&std::env::temp_dir(), "claims-retry-wait").unwrap();
    let ledger = Ledger::of(workspace.path());
    let _hooks = Hooks;
    let calls = Rc::new(Cell::new(0));
    let seen = Rc::clone(&calls);
    CREATE_FOLDER.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |path| {
            seen.set(seen.get() + 1);
            if seen.get() <= 3 {
                if create {
                    Err(io::Error::from(io::ErrorKind::NotFound))
                } else {
                    // The folder vanished before open: the real open must return NotFound.
                    Ok(())
                }
            } else {
                fs::create_dir(path)
            }
        }));
    });
    let pauses = Rc::new(RefCell::new(Vec::new()));
    let slept = Rc::clone(&pauses);
    SLEEP.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |delay| slept.borrow_mut().push(delay)));
    });

    let tracker = ledger.lock_creating_for(wait);
    assert_eq!(tracker.fault(), None);
    assert_eq!(calls.get(), 4);
    let pauses = pauses.borrow();
    if wait.is_zero() {
        assert!(pauses.is_empty(), "ZERO requested sleep: {pauses:?}");
    } else {
        assert_eq!(pauses.len(), 3, "each failed attempt must yield CPU");
        assert!(
            pauses
                .iter()
                .all(|delay| !delay.is_zero() && *delay <= Duration::from_micros(500))
        );
    }
}

#[test]
fn a_missing_folder_create_pauses_before_retrying() {
    transient_missing(true, Duration::from_secs(5));
}

#[test]
fn a_missing_lock_open_pauses_before_retrying() {
    transient_missing(false, Duration::from_secs(5));
}

#[test]
fn zero_wait_never_sleeps_on_either_missing_path() {
    transient_missing(true, Duration::ZERO);
    transient_missing(false, Duration::ZERO);
}

#[test]
fn a_missing_workspace_fails_closed_without_creating_parent_folders() {
    let workspace = Staging::create(&std::env::temp_dir(), "claims-missing-workspace").unwrap();
    let absent = workspace.path().join("missing");
    let ledger = Ledger::of(&absent);
    let tracker = ledger.lock_creating_for(Duration::ZERO);
    let fault = tracker
        .fault()
        .expect("a missing workspace cannot be locked");
    assert!(fault.contains("retry budget exhausted"), "{fault}");
    assert!(!absent.exists());
}
