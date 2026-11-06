//! BL112.3 (F025/F040): how long a **healthy** `recover` holds `ledger.lock`, and what the window's
//! tick costs while it does.
//!
//! `recover` keeps one `Tracker` across the `forget` of every handled orphan, and each `forget`
//! rewrites the whole ledger (`recover.rs`). With many orphans that is a long hold, and the tick
//! waits only `TICK_WAIT` (20 ms) for the lock. Reading the code cannot say how long "long" is,
//! so this measures it, with real files, real stamps and the real [`recover_with`]:
//!
//! * **hold run** (no ticks): an observer thread opens `ledger.lock` and asks for a shared lock
//!   every ~50 µs; a refusal means the exclusive holder is in. The contiguous refusals are the
//!   spans the lock was held, so `total` and `longest` are what `recover` kept the ledger for
//!   (an upper bound by the observer's granularity; its own brief locks can delay `recover` by at
//!   most one `LOCK_POLL`).
//! * **tick run** (no observer): a thread does what the tick does against the ledger, every ~1 ms:
//!   `try_lock` (open, read, reap), claim an entry of its own and accept it (a second write), all
//!   under the one lock, so a tick is a read plus two writes of the ledger. Ticks that start before
//!   `recover` are the control (same ledger, same load, nobody holding); ticks that start while it
//!   runs are the measurement. A refusal ("gave up after") counts, with how long it took to say it.
//! * `0` orphans is the control run, `1`, `10` and `100` are the bounded sizes (no 10 000-orphan
//!   flush soak), three replicas each.
//!
//! The 100 ms ceiling of the UI is asserted against the slowest tick **that ran during** the
//! recovery and is not relaxed: if it fails, the acceptance has to leave the tick (BL112.5).
//!
//! Not measured: orphans that are large (hashing happens **outside** the ledger lock, see
//! `recover.rs`), several recoveries at once, a window painting. The tests are `#[ignore]`d with a
//! reason: run on purpose under the GPU lock, `TESSEL_BUDGET_LOG=<file>` appends one JSON object
//! per sample: `cargo test -p tessel-editor-media --lib recover_budget -- --ignored --nocapture`.

use std::fs::{self, File, TryLockError};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tessel_editor_core::jobs::CancelToken;

use super::claims::{Ledger, Stamp, next_holder};
use super::recover::recover_with;
use crate::fsx::{Staging, hash_file};

/// The UI's ceiling for one tick. Fixed before any measurement; never raised to make a run pass.
const CEILING: Duration = Duration::from_millis(100);
const REPLICAS: usize = 3;
const SIZES: [usize; 4] = [0, 1, 10, 100];

fn emit(record: &Value) {
    use std::io::Write;
    eprintln!("BUDGET {record}");
    if let Some(path) = std::env::var_os("TESSEL_BUDGET_LOG")
        && let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path)
    {
        let _ = writeln!(file, "{record}");
    }
}

fn millis(elapsed: Duration) -> f64 {
    elapsed.as_secs_f64() * 1000.0
}

/// Nearest-rank quantile of an ascending list of milliseconds.
fn quantile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (sorted.len() as f64 * q).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

fn stats(mut values: Vec<f64>) -> Value {
    values.sort_by(f64::total_cmp);
    json!({
        "n": values.len(),
        "minMs": values.first().copied().unwrap_or(0.0),
        "p50Ms": quantile(&values, 0.5),
        "p99Ms": quantile(&values, 0.99),
        "maxMs": values.last().copied().unwrap_or(0.0),
    })
}

/// `count` files claimed by nobody: what a crashed process leaves. The holder is this process,
/// which gives the claim up without removing the file (`abandon`).
fn orphans(workspace: &Path, count: usize) -> Vec<PathBuf> {
    let root = workspace.join("scenes/s/assets");
    fs::create_dir_all(&root).unwrap();
    let root = fs::canonicalize(&root).unwrap();
    let holder = next_holder();
    let ledger = Ledger::of(workspace);
    let mut tracker = ledger.lock_creating();
    let mut paths = Vec::new();
    for index in 0..count {
        let path = root.join(format!("o{index}.bin"));
        fs::write(&path, format!("orphan budget file {index}")).unwrap();
        let metadata = fs::metadata(&path).unwrap();
        let (sha256, _) = hash_file(&path, &CancelToken::default(), |_| {}).unwrap();
        let stamp = Stamp {
            len: metadata.len(),
            device: metadata.dev(),
            inode: metadata.ino(),
            sha256,
        };
        tracker
            .create_file(holder, path.clone(), &root, Some(stamp))
            .unwrap();
        tracker.abandon(holder, &path);
        paths.push(path);
    }
    paths
}

/// The contiguous spans during which `lock` was held by someone else, until `stop`.
fn observe(lock: &Path, stop: &AtomicBool) -> Vec<Duration> {
    let mut spans = Vec::new();
    let mut since: Option<Instant> = None;
    while !stop.load(Ordering::Relaxed) {
        let held = match File::open(lock) {
            Ok(file) => match file.try_lock_shared() {
                Ok(()) => {
                    let _ = file.unlock();
                    false
                }
                Err(TryLockError::WouldBlock) => true,
                Err(TryLockError::Error(_)) => false,
            },
            // No lock file: no ledger at this moment.
            Err(_) => false,
        };
        match (held, since) {
            (true, None) => since = Some(Instant::now()),
            (false, Some(start)) => {
                spans.push(start.elapsed());
                since = None;
            }
            _ => {}
        }
        std::thread::sleep(Duration::from_micros(50));
    }
    if let Some(start) = since {
        spans.push(start.elapsed());
    }
    spans
}

#[derive(Debug)]
struct Tick {
    /// Started while `recover` ran (otherwise it is the control).
    during: bool,
    took: Duration,
    refused: bool,
    ledger_bytes: u64,
}

/// What the window's tick does to the ledger, every ~1 ms, until `stop`: `try_lock` (its fixed
/// budget), then claim an entry and accept it under the same lock.
fn tick(ledger: &Ledger, claims: &Path, running: &AtomicBool, stop: &AtomicBool) -> Vec<Tick> {
    let holder = next_holder();
    let root = PathBuf::from("/te-budget");
    let mut ticks = Vec::new();
    let mut index = 0u64;
    while !stop.load(Ordering::Relaxed) {
        let during = running.load(Ordering::Relaxed);
        let key = PathBuf::from(format!("/te-budget/tick-{index}"));
        index += 1;
        let started = Instant::now();
        let mut tracker = ledger.try_lock();
        let refused = tracker.fault().is_some();
        if !refused {
            tracker
                .create_file(holder, key.clone(), &root, None)
                .unwrap();
            tracker.accept(std::iter::once(key.as_path())).unwrap();
        }
        drop(tracker);
        ticks.push(Tick {
            during,
            took: started.elapsed(),
            refused,
            ledger_bytes: fs::metadata(claims).map_or(0, |meta| meta.len()),
        });
        std::thread::sleep(Duration::from_millis(1));
    }
    ticks
}

struct Run {
    wall: Duration,
    spans: Vec<Duration>,
    ticks: Vec<Tick>,
}

/// One healthy recovery of `count` orphans, observed (`hold`) or ticked (`ticks`), never both.
fn recover_once(count: usize, hold: bool, ticks: bool) -> Run {
    let dir = Staging::create(&std::env::temp_dir(), "recover-budget").unwrap();
    let paths = orphans(dir.path(), count);
    let ledger = Ledger::of(dir.path());
    // A claim of this process that stays: its lease is not let go of, and every write is real.
    let holder = next_holder();
    ledger
        .lock_creating()
        .create_file(
            holder,
            PathBuf::from("/te-budget/bystander"),
            Path::new("/te-budget"),
            None,
        )
        .unwrap();
    let canonical = fs::canonicalize(dir.path()).unwrap();
    let lock = canonical.join(".tessel-claims/ledger.lock");
    let claims = canonical.join(".tessel-claims/claims.json");
    let (running, stop) = (AtomicBool::new(false), AtomicBool::new(false));

    let (wall, spans, observed) = std::thread::scope(|scope| {
        let watcher = hold.then(|| scope.spawn(|| observe(&lock, &stop)));
        let ticker = ticks.then(|| scope.spawn(|| tick(&ledger, &claims, &running, &stop)));
        // Control ticks first: the same ledger and load, no recovery yet.
        std::thread::sleep(Duration::from_millis(30));
        running.store(true, Ordering::Relaxed);
        let started = Instant::now();
        let recovery = recover_with(dir.path(), &CancelToken::default(), &mut |_, _| {})
            .expect("a healthy recovery");
        let wall = started.elapsed();
        running.store(false, Ordering::Relaxed);
        assert_eq!(recovery.report.removed_files, count, "{recovery:?}");
        std::thread::sleep(Duration::from_millis(10));
        stop.store(true, Ordering::Relaxed);
        (
            wall,
            watcher
                .map(|handle| handle.join().unwrap())
                .unwrap_or_default(),
            ticker
                .map(|handle| handle.join().unwrap())
                .unwrap_or_default(),
        )
    });
    assert!(paths.iter().all(|path| !path.exists()), "every orphan went");
    assert_eq!(
        ledger.lock().entries().count(),
        1,
        "only the bystander is left claimed"
    );
    ledger
        .lock()
        .leave(holder, Path::new("/te-budget/bystander"));
    Run {
        wall,
        spans,
        ticks: observed,
    }
}

#[test]
#[ignore = "BL112.3 measurement of the ledger hold of a healthy recovery; run on purpose under the GPU lock with --ignored"]
fn a_healthy_recovery_holds_the_ledger_for_this_long() {
    for count in SIZES {
        for replica in 0..REPLICAS {
            let run = recover_once(count, true, false);
            let total: Duration = run.spans.iter().sum();
            let longest = run.spans.iter().max().copied().unwrap_or_default();
            emit(
                &json!({"measure": "recover-hold", "orphans": count, "replica": replica,
                "recoverWallMs": millis(run.wall), "holdTotalMs": millis(total),
                "holdLongestMs": millis(longest), "holdSpans": run.spans.len(),
                "tickBudgetMs": 20}),
            );
        }
    }
}

#[test]
#[ignore = "BL112.3 measurement of the tick during a healthy recovery; run on purpose under the GPU lock with --ignored"]
fn the_tick_during_a_healthy_recovery_stays_under_its_ceiling() {
    let mut worst_during = Duration::ZERO;
    for count in SIZES {
        let mut control = Vec::new();
        let mut during = Vec::new();
        let (mut refused, mut seen_during) = (0usize, 0usize);
        let mut ledger_bytes = 0;
        for replica in 0..REPLICAS {
            let run = recover_once(count, false, true);
            let in_recovery: Vec<&Tick> = run.ticks.iter().filter(|tick| tick.during).collect();
            let idle: Vec<&Tick> = run.ticks.iter().filter(|tick| !tick.during).collect();
            emit(&json!({"measure": "recover-tick-replica", "orphans": count,
                "replica": replica, "recoverWallMs": millis(run.wall),
                "ticksDuring": in_recovery.len(), "ticksIdle": idle.len(),
                "refusedDuring": in_recovery.iter().filter(|tick| tick.refused).count(),
                "maxDuringMs": in_recovery.iter().map(|tick| millis(tick.took)).fold(0.0, f64::max),
                "maxIdleMs": idle.iter().map(|tick| millis(tick.took)).fold(0.0, f64::max)}));
            ledger_bytes = ledger_bytes.max(
                run.ticks
                    .iter()
                    .map(|tick| tick.ledger_bytes)
                    .max()
                    .unwrap_or(0),
            );
            seen_during += in_recovery.len();
            refused += in_recovery.iter().filter(|tick| tick.refused).count();
            during.extend(in_recovery.iter().map(|tick| millis(tick.took)));
            control.extend(idle.iter().map(|tick| millis(tick.took)));
        }
        let longest = during.iter().copied().fold(0.0, f64::max);
        worst_during = worst_during.max(Duration::from_secs_f64(longest / 1000.0));
        emit(&json!({"measure": "recover-tick-summary", "orphans": count,
            "replicas": REPLICAS, "samplesDuring": seen_during, "refusedDuring": refused,
            "during": stats(during), "control": stats(control),
            "ledgerBytesMax": ledger_bytes, "ceilingMs": millis(CEILING)}));
    }
    assert!(
        worst_during < CEILING,
        "a tick took {worst_during:?} during a healthy recovery: over the 100 ms ceiling, so the \
         acceptance has to leave the tick (BL112.5)"
    );
}
