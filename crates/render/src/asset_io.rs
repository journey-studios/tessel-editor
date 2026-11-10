//! Bounded worker pool that reads the project's files for the viewport page, off the thread that received the request.
//!
//! The desktop WebView hands every `tessel://` request to a callback on the window's own thread. Answering it there means
//! the whole `File::open` / seek / `read_to_end` of a splat (tens of MiB) happens while the window cannot paint or take
//! input. This pool is the diagnostic alternative of BL157.1: the callback decides what the request refers to
//! ([`crate::host::resolve`], authorization included, on its own thread) and hands only the read to a worker, which answers
//! through the closure it was given.
//!
//! What is bounded, and what happens at the bound (nothing is dropped silently):
//!
//! * **Workers**: a fixed number of threads, created once. Never a thread per request.
//! * **Queue**: a fixed number of waiting jobs. A job submitted past it is *returned* to the caller with
//!   [`Rejection::Saturated`] so the caller answers it (HTTP 503) and the run is visibly invalid; it is never discarded.
//! * **Bytes held**: each worker holds at most the body of the one job it is reading (a ranged answer is capped at
//!   [`crate::host::MAX_RANGE_BODY`]; a plain `GET` is the whole file, exactly as in the synchronous path), so the
//!   pool holds at most `workers` bodies; a queued job is a path and a header.
//! * **Staleness**: [`AssetPool::invalidate`] (a project replaced, the page reloaded) makes every job submitted before it
//!   answer `410` without I/O if it had not started, and drops the body of one already read instead of delivering it.
//!   Each job's closure runs exactly once, whatever happens (including a panic while reading and the pool being dropped).
//!
//! The pool knows nothing about GPUI or the WebView: the caller's closure does the `Response` plumbing.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::host::{Served, read_file};

/// Workers of the pool: the engineering starting point fixed before the measurement (BL157.1), not tuned per run.
pub const DEFAULT_WORKERS: usize = 2;
/// Jobs that may wait for a worker, fixed for the same reason.
pub const DEFAULT_QUEUE: usize = 16;

/// How a job ended, for whoever traces it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Outcome {
    /// Which worker ran the job (0-based), `None` when the job never reached one (rejected).
    pub worker: Option<usize>,
    /// Submission to the worker taking it.
    pub queue_wait: Duration,
    /// The `File::open`..`read_to_end` of the job (zero when it was stale before it started).
    pub read: Duration,
    /// The pool was invalidated while the job waited or read: the answer is `410`, not the file.
    pub stale: bool,
    /// Jobs waiting (this one included) when it was submitted.
    pub depth_at_submit: usize,
}

/// A job's answer closure. It runs on a worker thread, once.
pub type Done = Box<dyn FnOnce(Served, Outcome) + Send>;

/// Why a job was not accepted. The job's [`Done`] is handed back with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    /// The queue was full.
    Saturated,
    /// The pool is shutting down.
    Closed,
}

/// What to read.
#[derive(Debug, Clone)]
pub struct ReadJob {
    pub file: PathBuf,
    pub range: Option<String>,
}

struct Message {
    job: ReadJob,
    done: Done,
    epoch: u64,
    submitted: Instant,
    depth: usize,
}

/// Counters, all monotonic except the live gauges. Reported by `render.status` and stable names of the experiment.
#[derive(Debug, Default)]
pub struct IoStats {
    submitted: AtomicU64,
    completed: AtomicU64,
    saturated: AtomicU64,
    stale: AtomicU64,
    panicked: AtomicU64,
    queued: AtomicUsize,
    in_flight: AtomicUsize,
    max_queued: AtomicUsize,
    max_in_flight: AtomicUsize,
    bytes_held: AtomicUsize,
    max_bytes_held: AtomicUsize,
}

impl IoStats {
    pub fn submitted(&self) -> u64 {
        self.submitted.load(Ordering::Relaxed)
    }

    pub fn completed(&self) -> u64 {
        self.completed.load(Ordering::Relaxed)
    }

    /// Jobs refused because the queue was full. Any above zero makes a run invalid.
    pub fn saturated(&self) -> u64 {
        self.saturated.load(Ordering::Relaxed)
    }

    pub fn stale(&self) -> u64 {
        self.stale.load(Ordering::Relaxed)
    }

    pub fn queued(&self) -> usize {
        self.queued.load(Ordering::Relaxed)
    }

    pub fn in_flight(&self) -> usize {
        self.in_flight.load(Ordering::Relaxed)
    }

    pub fn max_bytes_held(&self) -> usize {
        self.max_bytes_held.load(Ordering::Relaxed)
    }

    pub fn to_json(&self) -> Value {
        json!({
            "submitted": self.submitted(),
            "completed": self.completed(),
            "saturated": self.saturated(),
            "stale": self.stale(),
            "panicked": self.panicked.load(Ordering::Relaxed),
            "queued": self.queued(),
            "in_flight": self.in_flight(),
            "max_queued": self.max_queued.load(Ordering::Relaxed),
            "max_in_flight": self.max_in_flight.load(Ordering::Relaxed),
            "bytes_held": self.bytes_held.load(Ordering::Relaxed),
            "max_bytes_held": self.max_bytes_held(),
        })
    }
}

struct Shared {
    sender: SyncSender<Message>,
    epoch: AtomicU64,
    stats: IoStats,
    workers: usize,
    queue: usize,
}

/// The pool. Cloning shares it; the workers end when the last clone is dropped (without waiting for them).
#[derive(Clone)]
pub struct AssetPool {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for AssetPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssetPool")
            .field("workers", &self.shared.workers)
            .field("queue", &self.shared.queue)
            .finish()
    }
}

fn gone() -> Served {
    Served {
        status: 410,
        content_type: "text/plain; charset=utf-8",
        body: b"the project or the viewport changed before this file was read".to_vec(),
        content_range: None,
    }
}

impl AssetPool {
    /// `workers` threads and room for `queue` waiting jobs (both at least 1).
    pub fn new(workers: usize, queue: usize) -> Self {
        let workers = workers.max(1);
        let queue = queue.max(1);
        let (sender, receiver) = sync_channel::<Message>(queue);
        let shared = Arc::new(Shared {
            sender,
            epoch: AtomicU64::new(0),
            stats: IoStats::default(),
            workers,
            queue,
        });
        let receiver = Arc::new(Mutex::new(receiver));
        for index in 0..workers {
            let receiver = receiver.clone();
            // A worker holds the shared counters, not the sender: the channel closes when the last pool handle drops.
            let view = WorkerView {
                shared: Arc::downgrade(&shared),
            };
            let spawned = std::thread::Builder::new()
                .name(format!("tessel-asset-io-{index}"))
                .spawn(move || work(index, &receiver, &view));
            // A thread that cannot start leaves a smaller pool, never a hang: the queue still answers (or saturates).
            drop(spawned);
        }
        Self { shared }
    }

    pub fn workers(&self) -> usize {
        self.shared.workers
    }

    pub fn queue_capacity(&self) -> usize {
        self.shared.queue
    }

    pub fn stats(&self) -> &IoStats {
        &self.shared.stats
    }

    /// A project or page change: everything submitted until now is answered `410` instead of being applied.
    pub fn invalidate(&self) {
        self.shared.epoch.fetch_add(1, Ordering::SeqCst);
    }

    /// Hands `job` to a worker. Never blocks. On refusal the closure comes back so the caller answers the request
    /// itself (the pool never keeps a job it did not accept and never drops one).
    pub fn submit(&self, job: ReadJob, done: Done) -> Result<(), (Rejection, Done)> {
        let stats = &self.shared.stats;
        let depth = stats.queued.fetch_add(1, Ordering::SeqCst) + 1;
        let message = Message {
            job,
            done,
            epoch: self.shared.epoch.load(Ordering::SeqCst),
            submitted: Instant::now(),
            depth,
        };
        match self.shared.sender.try_send(message) {
            Ok(()) => {
                stats.submitted.fetch_add(1, Ordering::Relaxed);
                stats.max_queued.fetch_max(depth, Ordering::Relaxed);
                Ok(())
            }
            Err(TrySendError::Full(message)) => {
                stats.queued.fetch_sub(1, Ordering::SeqCst);
                stats.saturated.fetch_add(1, Ordering::Relaxed);
                Err((Rejection::Saturated, message.done))
            }
            Err(TrySendError::Disconnected(message)) => {
                stats.queued.fetch_sub(1, Ordering::SeqCst);
                Err((Rejection::Closed, message.done))
            }
        }
    }
}

/// What a worker may keep: counters and the epoch, but not the sender (so the channel can close).
struct WorkerView {
    shared: std::sync::Weak<Shared>,
}

fn work(index: usize, receiver: &Mutex<Receiver<Message>>, view: &WorkerView) {
    loop {
        let message = {
            let guard = receiver
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.recv()
        };
        let Ok(message) = message else {
            return;
        };
        run(index, message, view);
    }
}

fn run(index: usize, message: Message, view: &WorkerView) {
    let taken = Instant::now();
    let queue_wait = taken.saturating_duration_since(message.submitted);
    let shared = view.shared.upgrade();
    let epoch_now = || {
        shared
            .as_ref()
            .map_or(u64::MAX, |shared| shared.epoch.load(Ordering::SeqCst))
    };
    if let Some(shared) = &shared {
        let stats = &shared.stats;
        stats.queued.fetch_sub(1, Ordering::SeqCst);
        let now = stats.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        stats.max_in_flight.fetch_max(now, Ordering::Relaxed);
    }
    let stale_before = epoch_now() != message.epoch;
    let started = Instant::now();
    let mut served = if stale_before {
        gone()
    } else {
        catch_unwind(AssertUnwindSafe(|| {
            read_file(&message.job.file, message.job.range.as_deref())
        }))
        .unwrap_or_else(|_| {
            if let Some(shared) = &shared {
                shared.stats.panicked.fetch_add(1, Ordering::Relaxed);
            }
            Served {
                status: 500,
                content_type: "text/plain; charset=utf-8",
                body: b"the asset read panicked".to_vec(),
                content_range: None,
            }
        })
    };
    let read = if stale_before {
        Duration::ZERO
    } else {
        started.elapsed()
    };
    // The body that was just read is memory the pool holds from now on, even when the session that asked is already over.
    // The figure starts when the read returns: the buffer a worker is still filling (at most one per worker) is not in it.
    let account = |bytes: usize| {
        if let Some(shared) = &shared {
            let stats = &shared.stats;
            let now = stats.bytes_held.fetch_add(bytes, Ordering::SeqCst) + bytes;
            stats.max_bytes_held.fetch_max(now, Ordering::Relaxed);
        }
    };
    let release = |bytes: usize| {
        if let Some(shared) = &shared {
            shared.stats.bytes_held.fetch_sub(bytes, Ordering::SeqCst);
        }
    };
    let mut held = served.body.len();
    account(held);
    // A pool that was dropped has no epoch to match: what is still queued is stale, answered without I/O.
    let stale = stale_before || epoch_now() != message.epoch;
    if stale && !stale_before {
        // Read, but the session that asked is over: the bytes are dropped here, not delivered.
        served = gone();
        release(held);
        held = served.body.len();
        account(held);
    }
    if stale && let Some(shared) = &shared {
        shared.stats.stale.fetch_add(1, Ordering::Relaxed);
    }
    let outcome = Outcome {
        worker: Some(index),
        queue_wait,
        read,
        stale,
        depth_at_submit: message.depth,
    };
    // The closure runs exactly once, even if it panics (the worker survives it).
    let _ = catch_unwind(AssertUnwindSafe(|| (message.done)(served, outcome)));
    release(held);
    if let Some(shared) = &shared {
        let stats = &shared.stats;
        stats.in_flight.fetch_sub(1, Ordering::SeqCst);
        stats.completed.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{AssetAccess, Resolved, resolve, serve};
    use serde_json::json;
    use std::sync::mpsc;
    use std::time::Duration;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tessel-io-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn encode(path: &std::path::Path) -> String {
        path.to_string_lossy()
            .replace('/', "%2F")
            .replace(' ', "%20")
    }

    /// Runs a request through the pool and waits for its answer.
    fn pooled(
        pool: &AssetPool,
        dist: &std::path::Path,
        access: &AssetAccess,
        query: &str,
        range: Option<&str>,
    ) -> (Served, Option<Outcome>) {
        match resolve(dist, access, "/asset", query, range) {
            Resolved::Answer(served) => (served, None),
            Resolved::File { file, range } => {
                let (tx, rx) = mpsc::channel();
                pool.submit(
                    ReadJob { file, range },
                    Box::new(move |served, outcome| {
                        tx.send((served, outcome)).unwrap();
                    }),
                )
                .map_err(|(rejection, _)| rejection)
                .unwrap();
                let (served, outcome) = rx.recv_timeout(Duration::from_secs(20)).unwrap();
                (served, Some(outcome))
            }
        }
    }

    #[test]
    fn the_pool_answers_exactly_what_the_synchronous_path_answers() {
        let dir = scratch("same");
        let body: Vec<u8> = (0..(crate::host::MAX_RANGE_BODY + 12_345))
            .map(|index| (index % 251) as u8)
            .collect();
        std::fs::write(dir.join("big.ply"), &body).unwrap();
        std::fs::write(dir.join("small.png"), b"0123456789").unwrap();
        std::fs::write(dir.join("empty.png"), b"").unwrap();
        std::fs::write(dir.join("secret.txt"), b"nope").unwrap();
        let mut access = AssetAccess::default();
        for name in ["big.ply", "small.png", "empty.png", "gone.png"] {
            access.register(&json!({"source": {"type": "file", "path": dir.join(name)}}));
        }
        let pool = AssetPool::new(DEFAULT_WORKERS, DEFAULT_QUEUE);
        let cases: Vec<(String, Option<&str>)> = vec![
            ("big.ply".into(), None),
            ("big.ply".into(), Some("bytes=0-9")),
            ("big.ply".into(), Some("bytes=5-")),
            ("big.ply".into(), Some("bytes=-100")),
            ("big.ply".into(), Some("bytes=-100000000")),
            ("big.ply".into(), Some("bytes=999999999-")),
            ("big.ply".into(), Some("bytes=9-3")),
            ("big.ply".into(), Some("lines=1-2")),
            ("small.png".into(), None),
            ("small.png".into(), Some("bytes=2-5")),
            ("small.png".into(), Some("bytes=50-60")),
            ("empty.png".into(), None),
            ("empty.png".into(), Some("bytes=0-0")),
            ("gone.png".into(), None),
            ("secret.txt".into(), None),
        ];
        for (name, range) in cases {
            let query = format!("p={}", encode(&dir.join(&name)));
            let sync = serve(&dir, &access, "/asset", &query, range);
            let (async_answer, _) = pooled(&pool, &dir, &access, &query, range);
            assert_eq!(
                sync, async_answer,
                "{name} {range:?}: the pool must answer what `serve` answers (status, headers, bytes)"
            );
        }
        // The refusal and the missing file are the statuses of the path they replace.
        let secret = format!("p={}", encode(&dir.join("secret.txt")));
        assert_eq!(pooled(&pool, &dir, &access, &secret, None).0.status, 403);
        let missing = format!("p={}", encode(&dir.join("gone.png")));
        assert_eq!(pooled(&pool, &dir, &access, &missing, None).0.status, 404);
        let traversal = format!("p={}", encode(&dir.join("x/../secret.txt")));
        assert_eq!(pooled(&pool, &dir, &access, &traversal, None).0.status, 403);
    }

    #[cfg(unix)]
    fn fifo(dir: &std::path::Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        let status = std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success(), "mkfifo");
        path
    }

    /// Opens the write side of a fifo without blocking: readers waiting in `open` proceed and read end-of-file.
    #[cfg(unix)]
    fn release(path: &std::path::Path) -> std::fs::File {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap()
    }

    fn wait_until(what: &str, mut ready: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !ready() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::yield_now();
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_full_queue_returns_the_job_and_everything_accepted_is_answered_once() {
        let dir = scratch("full");
        let held = fifo(&dir, "held.bin");
        let pool = AssetPool::new(2, 3);
        let (tx, rx) = mpsc::channel::<u16>();
        let submit = |pool: &AssetPool| {
            let tx = tx.clone();
            pool.submit(
                ReadJob {
                    file: held.clone(),
                    range: None,
                },
                Box::new(move |served, _| tx.send(served.status).unwrap()),
            )
        };
        // Two workers block in `open` of the fifo; three more wait in the queue.
        for _ in 0..2 {
            assert!(submit(&pool).is_ok());
        }
        wait_until("both workers to hold a job", || {
            pool.stats().in_flight() == 2
        });
        for _ in 0..3 {
            assert!(submit(&pool).is_ok());
        }
        assert_eq!(pool.stats().queued(), 3);
        // The sixth does not fit: it is handed back, counted, and nothing else changed.
        let Err((rejection, done)) = submit(&pool) else {
            panic!("the queue was full");
        };
        assert_eq!(rejection, Rejection::Saturated);
        assert_eq!(pool.stats().saturated(), 1);
        assert_eq!(pool.stats().queued(), 3);
        assert_eq!(pool.stats().in_flight(), 2);
        // The caller answers the refused one itself, explicitly.
        done(
            Served {
                status: 503,
                content_type: "text/plain; charset=utf-8",
                body: b"busy".to_vec(),
                content_range: None,
            },
            Outcome {
                worker: None,
                queue_wait: Duration::ZERO,
                read: Duration::ZERO,
                stale: false,
                depth_at_submit: 0,
            },
        );
        assert_eq!(rx.recv().unwrap(), 503);
        let writer = release(&held);
        let mut answers = Vec::new();
        for _ in 0..5 {
            answers.push(rx.recv_timeout(Duration::from_secs(20)).unwrap());
        }
        drop(writer);
        assert_eq!(answers, vec![200; 5]);
        wait_until("the pool to settle", || {
            pool.stats().completed() == 5 && pool.stats().in_flight() == 0
        });
        assert!(rx.try_recv().is_err(), "no job answered twice");
        let stats = pool.stats().to_json();
        assert_eq!(stats["submitted"], 5);
        assert_eq!(stats["max_in_flight"], 2);
        assert!(stats["max_queued"].as_u64().unwrap() <= 5);
    }

    #[cfg(unix)]
    #[test]
    fn invalidating_answers_waiting_jobs_gone_without_reading_and_drops_the_body_of_the_one_in_flight()
     {
        let dir = scratch("stale");
        let held = fifo(&dir, "held.bin");
        let real = dir.join("real.bin");
        std::fs::write(&real, vec![7u8; 1 << 20]).unwrap();
        let pool = AssetPool::new(1, 4);
        let (tx, rx) = mpsc::channel::<(&'static str, Served, Outcome)>();
        let send = |label: &'static str, file: PathBuf| {
            let tx = tx.clone();
            pool.submit(
                ReadJob { file, range: None },
                Box::new(move |served, outcome| tx.send((label, served, outcome)).unwrap()),
            )
            .map_err(|(rejection, _)| rejection)
            .unwrap();
        };
        send("in_flight", held.clone());
        wait_until("the worker to hold the first job", || {
            pool.stats().in_flight() == 1
        });
        send("waiting", real.clone());
        pool.invalidate();
        send("after", real.clone());
        let writer = release(&held);
        let mut by_label = std::collections::HashMap::new();
        for _ in 0..3 {
            let (label, served, outcome) = rx.recv_timeout(Duration::from_secs(20)).unwrap();
            by_label.insert(label, (served, outcome));
        }
        drop(writer);
        let (in_flight, in_flight_outcome) = &by_label["in_flight"];
        assert_eq!(
            in_flight.status, 410,
            "read while the session changed: dropped"
        );
        assert!(in_flight_outcome.stale);
        let (waiting, waiting_outcome) = &by_label["waiting"];
        assert_eq!(waiting.status, 410);
        assert!(waiting_outcome.stale);
        assert_eq!(
            waiting_outcome.read,
            Duration::ZERO,
            "a job that was stale before it started is not read"
        );
        let (after, after_outcome) = &by_label["after"];
        assert_eq!(
            after.status, 200,
            "a job submitted after the change is the new session's"
        );
        assert_eq!(after.body.len(), 1 << 20);
        assert!(!after_outcome.stale);
        assert_eq!(pool.stats().stale(), 2);
    }

    #[test]
    fn many_jobs_hold_at_most_one_body_per_worker() {
        let dir = scratch("bytes");
        let size = 3 << 20;
        std::fs::write(dir.join("a.bin"), vec![1u8; size]).unwrap();
        let pool = AssetPool::new(2, 16);
        let (tx, rx) = mpsc::channel();
        let file = dir.join("a.bin");
        for _ in 0..12 {
            let tx = tx.clone();
            pool.submit(
                ReadJob {
                    file: file.clone(),
                    range: None,
                },
                Box::new(move |served, _| tx.send(served.body.len()).unwrap()),
            )
            .map_err(|(rejection, _)| rejection)
            .unwrap();
        }
        for _ in 0..12 {
            assert_eq!(rx.recv_timeout(Duration::from_secs(20)).unwrap(), size);
        }
        wait_until("the pool to settle", || pool.stats().in_flight() == 0);
        assert!(
            pool.stats().max_bytes_held() <= 2 * size,
            "two workers hold at most two bodies, held {}",
            pool.stats().max_bytes_held()
        );
        assert_eq!(pool.stats().to_json()["bytes_held"], 0);
    }

    #[test]
    fn a_panicking_answer_does_not_kill_the_worker_and_dropping_the_pool_does_not_wait() {
        let dir = scratch("panic");
        std::fs::write(dir.join("a.bin"), b"abc").unwrap();
        let pool = AssetPool::new(1, 4);
        let file = dir.join("a.bin");
        pool.submit(
            ReadJob {
                file: file.clone(),
                range: None,
            },
            Box::new(|_, _| panic!("a bad closure")),
        )
        .map_err(|(rejection, _)| rejection)
        .unwrap();
        let (tx, rx) = mpsc::channel();
        pool.submit(
            ReadJob { file, range: None },
            Box::new(move |served, _| tx.send(served.body).unwrap()),
        )
        .map_err(|(rejection, _)| rejection)
        .unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(20)).unwrap(), b"abc");
        let started = Instant::now();
        drop(pool);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn jobs_left_when_the_pool_is_dropped_still_answer_once_and_stale() {
        let dir = scratch("drop");
        std::fs::write(dir.join("a.bin"), b"abc").unwrap();
        let file = dir.join("a.bin");
        let (tx, rx) = mpsc::channel();
        {
            let pool = AssetPool::new(1, 8);
            for _ in 0..4 {
                let tx = tx.clone();
                pool.submit(
                    ReadJob {
                        file: file.clone(),
                        range: None,
                    },
                    Box::new(move |served, _| tx.send(served.status).unwrap()),
                )
                .map_err(|(rejection, _)| rejection)
                .unwrap();
            }
        }
        drop(tx);
        // Each of the four is answered exactly once; whichever had not started answers `410` and not the file.
        let statuses: Vec<u16> = rx.iter().take(4).collect();
        assert_eq!(statuses.len(), 4);
        assert!(statuses.iter().all(|status| [200, 410].contains(status)));
    }
}
