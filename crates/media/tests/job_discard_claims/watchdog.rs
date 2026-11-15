//! A bounded escape from a regression that blocks the parent on its own peer's ledger lock.

#[cfg(unix)]
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::Peer;

/// Borrow the owned child handle until the work and its watchdog have both finished. On timeout,
/// resume on Unix, then kill and reap on every platform: resuming alone does not release the
/// file the peer holds while waiting for its next stdin command.
pub(super) fn with_peer<T>(peer: &mut Peer, budget: Duration, work: impl FnOnce() -> T) -> T {
    let done = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let child = &mut peer.child;
        let finished = &done;
        let watchdog = scope.spawn(move || {
            let deadline = Instant::now() + budget;
            loop {
                if finished.load(Ordering::SeqCst) {
                    return;
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                std::thread::sleep(remaining.min(Duration::from_millis(50)));
            }
            #[cfg(unix)]
            let _ = Command::new("kill")
                .args(["-CONT", &child.id().to_string()])
                .status();
            let _ = child.kill();
            child.wait().expect("reap the watchdog's own peer");
        });
        let answer = work();
        done.store(true, Ordering::SeqCst);
        watchdog.join().expect("the peer watchdog");
        answer
    })
}

#[test]
fn an_expired_watchdog_terminates_its_peer_and_releases_the_real_ledger_lock() {
    let world = super::world("watchdog-expired");
    let _owner = world.bystander();
    let mut peer = Peer::spawn(&world);
    assert_eq!(peer.ask("hold_ledger")["held"], true);
    super::signal("STOP", peer.pid);
    let path = world.workspace.join(".tessel-claims/ledger.lock");
    let (sender, released) = std::sync::mpsc::channel();
    let waiter = std::thread::spawn(move || {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap();
        file.lock().unwrap();
        let _ = sender.send(());
    });
    let answer = with_peer(&mut peer, Duration::from_millis(20), || {
        released.recv_timeout(Duration::from_secs(5))
    });
    let terminated = peer
        .child
        .try_wait()
        .unwrap()
        .is_some_and(|status| !status.success());
    // Cleanup precedes assertions too: a broken watchdog fails in five seconds, without leaving
    // its peer or blocked waiter behind.
    let _ = peer.child.kill();
    peer.child.wait().unwrap();
    waiter.join().unwrap();
    assert!(
        answer.is_ok(),
        "the watchdog did not release the ledger lock: {answer:?}"
    );
    assert!(
        terminated,
        "the peer must have been terminated by the watchdog"
    );
}

#[test]
fn finished_work_cancels_the_watchdog_and_leaves_the_peer_alive() {
    let world = super::world("watchdog-cancelled");
    let mut peer = Peer::spawn(&world);
    with_peer(&mut peer, Duration::from_secs(20), || ());
    assert!(peer.child.try_wait().unwrap().is_none());
    peer.quit();
}
