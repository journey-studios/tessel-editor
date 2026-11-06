//! Long-running work as observable, cancellable jobs.
//!
//! A job has an ID, a kind, a state machine (`queued → running → succeeded|failed|cancelled`),
//! a stage with `done/total` units, warnings and a typed error. Callers poll [`JobRegistry::get`]
//! from any surface (library, CLI, UI): nothing depends on a window. Cancellation is
//! cooperative and *acknowledged*: `cancel` flips a token and the state becomes `cancelled`
//! only when the worker observed it and finished its cleanup.
//!
//! Concurrency is bounded (`max_running`), so a large import cannot starve an export.
//!
//! A job that produced a result for the project holds it ([`ResultState::Pending`]) until a
//! caller applies it or discards it (BL094). `prune` never drops a job that still holds a result
//! or is discarding one, and [`JobRegistry::begin_discard`] is the single atomic gate that
//! refuses an active, applied or already discarded job with a typed error.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::state::CommandError;

pub type JobId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Import,
    Hash,
    Probe,
    /// Read-only checks that touch files or tools (export and package preflight, package verify).
    Preflight,
    Proxy,
    Package,
    Restore,
    ProjectImport,
    VideoExport,
    /// Background removal of a finished result nobody applied (`job.discard`, BL094). It is not
    /// cancellable: it keeps the ownership of the files until it has taken them back.
    Discard,
}

impl JobKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Import => "import",
            Self::Hash => "hash",
            Self::Probe => "probe",
            Self::Preflight => "preflight",
            Self::Proxy => "proxy",
            Self::Package => "package",
            Self::Restore => "restore",
            Self::ProjectImport => "project-import",
            Self::VideoExport => "video-export",
            Self::Discard => "discard",
        }
    }

    pub fn is_cancellable(self) -> bool {
        !matches!(self, Self::Discard)
    }
}

/// What became of the result a job produced for the project. Part of the job's observable state
/// (`result_state` in `job.get`/`job.list`).
///
/// ```text
/// none ──(body keeps a result)──> pending ──apply──> applied
///                                    └──job.discard──> discarding ──> discarded
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResultState {
    /// The job produced nothing to apply (or failed, or was cancelled): nothing to discard.
    #[default]
    None,
    /// The job finished (or is finishing) with a result that waits to be applied or discarded.
    Pending,
    /// A discard job is taking the result's files back; the result still owns them.
    Discarding,
    Discarded,
    Applied,
}

impl ResultState {
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Pending => "pending",
            Self::Discarding => "discarding",
            Self::Discarded => "discarded",
            Self::Applied => "applied",
        }
    }

    /// A result in this state must outlive `prune`: dropping the record would lose its owner.
    pub fn is_held(self) -> bool {
        matches!(self, Self::Pending | Self::Discarding)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl JobState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn is_final(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

/// Typed failure of a job. `code` values are stable and shared with command errors.
#[derive(Debug, Clone, PartialEq)]
pub struct JobError {
    pub code: String,
    pub message: String,
    pub details: Option<Value>,
}

impl JobError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn cancelled() -> Self {
        Self::new("cancelled", "The job was cancelled")
    }

    pub fn is_cancelled(&self) -> bool {
        self.code == "cancelled"
    }

    pub fn to_json(&self) -> Value {
        let mut value = json!({ "code": self.code, "message": self.message });
        if let Some(details) = &self.details {
            value["details"] = details.clone();
        }
        value
    }
}

impl std::fmt::Display for JobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for JobError {}

impl From<CommandError> for JobError {
    fn from(error: CommandError) -> Self {
        let mut mapped = Self::new(error.code, error.message);
        mapped.details = error.details.map(|details| *details);
        mapped
    }
}

impl From<JobError> for CommandError {
    fn from(error: JobError) -> Self {
        // `code` is `&'static str` in CommandError; unknown codes keep their text in `details`.
        let code = match error.code.as_str() {
            "cancelled" => "cancelled",
            "io_error" => "io_error",
            "disk_full" => "disk_full",
            "integrity_error" => "integrity_error",
            "tool_unavailable" => "tool_unavailable",
            "unsupported_format" => "unsupported_format",
            "asset_unreadable" => "asset_unreadable",
            "preflight_failed" => "preflight_failed",
            "encode_failed" => "encode_failed",
            "render_failed" => "render_failed",
            "backend_unavailable" => "backend_unavailable",
            "invalid_package" => "invalid_package",
            "verification_failed" => "verification_failed",
            "destination_exists" => "destination_exists",
            "permission_denied" => "permission_denied",
            // The bundle importer's vocabulary (`scene.attach`), so the same refusal reads the
            // same whether it came from the command or from the job that prepared it.
            "bundle_unreadable" => "bundle_unreadable",
            "bundle_too_large" => "bundle_too_large",
            "invalid_json" => "invalid_json",
            "invalid_bundle" => "invalid_bundle",
            "resource_changed" => "resource_changed",
            "resource_missing" => "resource_missing",
            _ => "job_failed",
        };
        let mut mapped = Self::new(code, error.message);
        let mut details = error.details.unwrap_or_else(|| json!({}));
        if code == "job_failed" {
            details["job_code"] = json!(error.code);
        }
        mapped.details = Some(Box::new(details));
        mapped
    }
}

#[derive(Debug, Clone)]
pub struct JobInfo {
    pub id: JobId,
    pub kind: JobKind,
    pub label: String,
    pub state: JobState,
    pub stage: String,
    pub done: u64,
    pub total: u64,
    pub warnings: Vec<String>,
    pub error: Option<JobError>,
    pub result: Option<Value>,
    pub cancel_requested: bool,
    /// What became of the result of this job (BL094).
    pub result_state: ResultState,
    pub created_ms: u64,
    pub started_ms: Option<u64>,
    pub finished_ms: Option<u64>,
}

impl JobInfo {
    pub fn progress(&self) -> Option<f64> {
        (self.total > 0).then(|| (self.done as f64 / self.total as f64).clamp(0.0, 1.0))
    }

    /// ETA in seconds from the average speed so far: an estimate, `None` until measurable.
    pub fn eta_seconds(&self, now_ms: u64) -> Option<f64> {
        let started = self.started_ms?;
        let progress = self.progress().filter(|p| *p > 0.0 && *p < 1.0)?;
        let elapsed = now_ms.saturating_sub(started) as f64 / 1000.0;
        Some(elapsed * (1.0 - progress) / progress)
    }

    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id, "kind": self.kind.label(), "label": self.label,
            "state": self.state.label(), "stage": self.stage,
            "done": self.done, "total": self.total, "progress": self.progress(),
            "eta_seconds": self.eta_seconds(now_ms()),
            "warnings": self.warnings, "error": self.error.as_ref().map(JobError::to_json),
            "result": self.result, "cancel_requested": self.cancel_requested,
            "result_state": self.result_state.label(),
            "created_ms": self.created_ms, "started_ms": self.started_ms,
            "finished_ms": self.finished_ms,
        })
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

/// Flag a worker polls. Cloning shares the flag.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    pub fn check(&self) -> Result<(), JobError> {
        if self.is_cancelled() {
            Err(JobError::cancelled())
        } else {
            Ok(())
        }
    }
}

struct Slot {
    info: JobInfo,
    token: CancelToken,
}

#[derive(Default)]
struct Inner {
    jobs: Vec<Slot>,
    running: usize,
    next: u64,
}

struct Shared {
    inner: Mutex<Inner>,
    changed: Condvar,
    max_running: usize,
}

/// Registry of jobs of one process. Cloning shares the registry.
#[derive(Clone)]
pub struct JobRegistry {
    shared: Arc<Shared>,
}

/// Handle a job body uses to report progress and observe cancellation.
#[derive(Clone)]
pub struct JobContext {
    id: JobId,
    token: CancelToken,
    registry: JobRegistry,
}

impl JobContext {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn token(&self) -> &CancelToken {
        &self.token
    }

    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    /// `Err(cancelled)` once cancellation was requested: bodies use `ctx.check()?` between units.
    pub fn check(&self) -> Result<(), JobError> {
        self.token.check()
    }

    pub fn stage(&self, stage: &str, total: u64) {
        self.registry.update(&self.id, |info| {
            info.stage = stage.to_string();
            info.done = 0;
            info.total = total;
        });
    }

    pub fn progress(&self, done: u64) {
        self.registry.update(&self.id, |info| info.done = done);
    }

    pub fn advance(&self, by: u64) {
        self.registry.update(&self.id, |info| info.done += by);
    }

    pub fn warn(&self, message: impl Into<String>) {
        let message = message.into();
        self.registry
            .update(&self.id, |info| info.warnings.push(message));
    }

    /// The body is about to return a result that waits to be applied or discarded: the job is
    /// not prunable until one of the two happens. A failure or cancellation clears it again.
    pub fn hold_result(&self) {
        self.registry.update(&self.id, |info| {
            info.result_state = ResultState::Pending;
        });
    }
}

impl JobRegistry {
    pub fn new(max_running: usize) -> Self {
        Self {
            shared: Arc::new(Shared {
                inner: Mutex::new(Inner::default()),
                changed: Condvar::new(),
                max_running: max_running.max(1),
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.shared
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn update(&self, id: &str, change: impl FnOnce(&mut JobInfo)) {
        let mut inner = self.lock();
        if let Some(slot) = inner.jobs.iter_mut().find(|slot| slot.info.id == id) {
            change(&mut slot.info);
        }
        drop(inner);
        self.shared.changed.notify_all();
    }

    fn register(&self, kind: JobKind, label: &str) -> (JobId, CancelToken) {
        let mut inner = self.lock();
        inner.next += 1;
        let id = format!("job-{}", inner.next);
        let token = CancelToken::new();
        inner.jobs.push(Slot {
            info: JobInfo {
                id: id.clone(),
                kind,
                label: label.to_string(),
                state: JobState::Queued,
                stage: "queued".into(),
                done: 0,
                total: 0,
                warnings: Vec::new(),
                error: None,
                result: None,
                cancel_requested: false,
                result_state: ResultState::None,
                created_ms: now_ms(),
                started_ms: None,
                finished_ms: None,
            },
            token: token.clone(),
        });
        (id, token)
    }

    /// Blocks until a running slot is free or the job was cancelled while queued.
    fn acquire(&self, id: &str, token: &CancelToken) -> bool {
        let mut inner = self.lock();
        loop {
            if token.is_cancelled() {
                return false;
            }
            if inner.running < self.shared.max_running {
                inner.running += 1;
                return true;
            }
            let (guard, _) = self
                .shared
                .changed
                .wait_timeout(inner, Duration::from_millis(50))
                .unwrap_or_else(|poison| poison.into_inner());
            inner = guard;
            let _ = id;
        }
    }

    fn finish(&self, id: &str, outcome: Result<Value, JobError>, held_slot: bool) {
        let mut inner = self.lock();
        if held_slot {
            inner.running = inner.running.saturating_sub(1);
        }
        if let Some(slot) = inner.jobs.iter_mut().find(|slot| slot.info.id == id) {
            let info = &mut slot.info;
            info.finished_ms = Some(now_ms());
            match outcome {
                Ok(result) => {
                    info.state = JobState::Succeeded;
                    info.stage = "done".into();
                    info.done = info.total.max(info.done);
                    info.result = Some(result);
                }
                Err(error) if error.is_cancelled() => {
                    info.state = JobState::Cancelled;
                    info.stage = "cancelled".into();
                    info.error = Some(error);
                    info.result_state = ResultState::None;
                }
                Err(error) => {
                    info.state = JobState::Failed;
                    info.error = Some(error);
                    info.result_state = ResultState::None;
                }
            }
        }
        drop(inner);
        self.shared.changed.notify_all();
    }

    /// Runs `body` on a worker thread and returns its ID immediately.
    pub fn spawn<F>(&self, kind: JobKind, label: &str, body: F) -> JobId
    where
        F: FnOnce(&JobContext) -> Result<Value, JobError> + Send + 'static,
    {
        let (id, token) = self.register(kind, label);
        let registry = self.clone();
        let context = JobContext {
            id: id.clone(),
            token: token.clone(),
            registry: registry.clone(),
        };
        std::thread::spawn(move || {
            if !registry.acquire(&context.id, &token) {
                registry.finish(&context.id, Err(JobError::cancelled()), false);
                return;
            }
            registry.update(&context.id, |info| {
                info.state = JobState::Running;
                info.stage = "running".into();
                info.started_ms = Some(now_ms());
            });
            // A panic must end the job as failed instead of leaving it `running` forever.
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body(&context)))
                .unwrap_or_else(|_| Err(JobError::new("job_failed", "The job panicked")));
            registry.finish(&context.id, outcome, true);
        });
        id
    }

    /// Runs `body` on the calling thread but still records it as a job (CLI one-shot calls).
    pub fn run_inline<F>(
        &self,
        kind: JobKind,
        label: &str,
        body: F,
    ) -> (JobId, Result<Value, JobError>)
    where
        F: FnOnce(&JobContext) -> Result<Value, JobError>,
    {
        let (id, token) = self.register(kind, label);
        let context = JobContext {
            id: id.clone(),
            token,
            registry: self.clone(),
        };
        self.update(&id, |info| {
            info.state = JobState::Running;
            info.stage = "running".into();
            info.started_ms = Some(now_ms());
        });
        let outcome = body(&context);
        self.finish(&id, outcome.clone(), false);
        (id, outcome)
    }

    pub fn get(&self, id: &str) -> Option<JobInfo> {
        self.lock()
            .jobs
            .iter()
            .find(|slot| slot.info.id == id)
            .map(|slot| slot.info.clone())
    }

    pub fn list(&self) -> Vec<JobInfo> {
        self.lock()
            .jobs
            .iter()
            .map(|slot| slot.info.clone())
            .collect()
    }

    pub fn active(&self) -> Vec<JobInfo> {
        self.list()
            .into_iter()
            .filter(|info| !info.state.is_final())
            .collect()
    }

    /// Requests cancellation. The job keeps `running` (with `cancel_requested`) until its body
    /// observes the token and returns; that is the acknowledgement.
    pub fn cancel(&self, id: &str) -> Result<JobInfo, CommandError> {
        let mut inner = self.lock();
        let slot = inner
            .jobs
            .iter_mut()
            .find(|slot| slot.info.id == id)
            .ok_or_else(|| CommandError::unknown_element("job", id))?;
        if slot.info.state.is_final() {
            return Err(CommandError::new(
                "job_finished",
                format!("Job `{id}` already {}", slot.info.state.label()),
            ));
        }
        if !slot.info.kind.is_cancellable() {
            return Err(CommandError::new(
                "job_not_cancellable",
                format!(
                    "Job `{id}` is a {} job: it keeps the files of the result until it has taken them back",
                    slot.info.kind.label()
                ),
            ));
        }
        slot.token.cancel();
        slot.info.cancel_requested = true;
        let info = slot.info.clone();
        drop(inner);
        self.shared.changed.notify_all();
        Ok(info)
    }

    pub fn cancel_all(&self) -> usize {
        let ids: Vec<JobId> = self.active().into_iter().map(|info| info.id).collect();
        ids.iter().filter(|id| self.cancel(id).is_ok()).count()
    }

    /// Waits for a final state; `None` on timeout.
    pub fn wait(&self, id: &str, timeout: Duration) -> Option<JobInfo> {
        let deadline = Instant::now() + timeout;
        let mut inner = self.lock();
        loop {
            let info = inner
                .jobs
                .iter()
                .find(|slot| slot.info.id == id)?
                .info
                .clone();
            if info.state.is_final() {
                return Some(info);
            }
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            let (guard, _) = self
                .shared
                .changed
                .wait_timeout(inner, (deadline - now).min(Duration::from_millis(100)))
                .unwrap_or_else(|poison| poison.into_inner());
            inner = guard;
        }
    }

    /// Drops finished jobs beyond `keep`, oldest first, so a long session does not grow forever.
    /// A job that still holds a result nobody applied or discarded, or whose result is being
    /// discarded, is neither dropped nor counted: forgetting it would lose the owner of its
    /// files (BL094).
    pub fn prune(&self, keep: usize) {
        let mut inner = self.lock();
        let mut finished: VecDeque<usize> = inner
            .jobs
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.info.state.is_final() && !slot.info.result_state.is_held())
            .map(|(index, _)| index)
            .collect();
        while finished.len() > keep {
            if let Some(index) = finished.pop_front() {
                inner.jobs.remove(index);
                finished.iter_mut().for_each(|next| *next -= 1);
            }
        }
    }

    /// The caller consumed the pending result of `id`: `applied` when the project took it,
    /// otherwise it was refused and is gone (discarded). Only a result that is still pending
    /// moves; one a discard already claimed stays with the discard (`false`).
    pub fn settle(&self, id: &str, applied: bool) -> bool {
        let mut inner = self.lock();
        let Some(slot) = inner.jobs.iter_mut().find(|slot| slot.info.id == id) else {
            return false;
        };
        let pending = slot.info.result_state == ResultState::Pending;
        if pending {
            slot.info.result_state = if applied {
                ResultState::Applied
            } else {
                ResultState::Discarded
            };
        }
        drop(inner);
        self.shared.changed.notify_all();
        pending
    }

    /// Claims the result of `id` for a discard, atomically: of two concurrent callers exactly one
    /// wins, and an apply can no longer take the result. Refusals are typed and change nothing:
    /// `unknown_element`, `job_running` (cancel it and wait for the end first), `job_applied`,
    /// `job_discarding`, `job_discarded`, `job_nothing_to_discard` (it produced no result, or
    /// failed or was cancelled).
    pub fn begin_discard(&self, id: &str) -> Result<JobInfo, CommandError> {
        let mut inner = self.lock();
        let slot = inner
            .jobs
            .iter_mut()
            .find(|slot| slot.info.id == id)
            .ok_or_else(|| CommandError::unknown_element("job", id))?;
        if !slot.info.state.is_final() {
            return Err(CommandError::new(
                "job_running",
                format!(
                    "Job `{id}` is still {}: cancel it and wait for its end before discarding",
                    slot.info.state.label()
                ),
            ));
        }
        let refusal = match slot.info.result_state {
            ResultState::Pending => None,
            ResultState::Applied => Some((
                "job_applied",
                "its result was already applied to the project",
            )),
            ResultState::Discarding => Some(("job_discarding", "its result is being discarded")),
            ResultState::Discarded => Some(("job_discarded", "its result was already discarded")),
            ResultState::None => Some((
                "job_nothing_to_discard",
                "it produced no result (it failed, was cancelled or changes nothing)",
            )),
        };
        if let Some((code, why)) = refusal {
            return Err(CommandError::new(code, format!("Job `{id}`: {why}")));
        }
        slot.info.result_state = ResultState::Discarding;
        let info = slot.info.clone();
        drop(inner);
        self.shared.changed.notify_all();
        Ok(info)
    }

    /// The discard of `id` ended: its result no longer exists.
    pub fn finish_discard(&self, id: &str) {
        self.update(id, |info| info.result_state = ResultState::Discarded);
    }

    pub fn to_json(&self) -> Value {
        json!({ "jobs": self.list().iter().map(JobInfo::to_json).collect::<Vec<_>>() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_reports_progress_and_succeeds() {
        let registry = JobRegistry::new(2);
        let id = registry.spawn(JobKind::Hash, "hash", |ctx| {
            ctx.stage("reading", 4);
            for _ in 0..4 {
                ctx.advance(1);
            }
            ctx.warn("looked odd");
            Ok(json!({ "done": true }))
        });
        let info = registry.wait(&id, Duration::from_secs(5)).unwrap();
        assert_eq!(info.state, JobState::Succeeded);
        assert_eq!((info.done, info.total), (4, 4));
        assert_eq!(info.warnings, ["looked odd"]);
        assert_eq!(info.result.unwrap()["done"], true);
        assert_eq!(registry.get(&id).unwrap().progress(), Some(1.0));
    }

    #[test]
    fn cancellation_is_acknowledged_by_the_worker_and_cleanup_runs() {
        let registry = JobRegistry::new(1);
        let cleaned = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cleaned);
        let id = registry.spawn(JobKind::Import, "slow", move |ctx| {
            ctx.stage("copy", 1000);
            loop {
                if let Err(error) = ctx.check() {
                    flag.store(true, Ordering::SeqCst);
                    return Err(error);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        while registry.get(&id).unwrap().state != JobState::Running {
            std::thread::sleep(Duration::from_millis(2));
        }
        let requested = registry.cancel(&id).unwrap();
        assert!(requested.cancel_requested);
        let info = registry.wait(&id, Duration::from_secs(5)).unwrap();
        assert_eq!(info.state, JobState::Cancelled);
        assert!(
            cleaned.load(Ordering::SeqCst),
            "the body ran its cleanup before the ack"
        );
        assert_eq!(registry.cancel(&id).unwrap_err().code, "job_finished");
        assert_eq!(
            registry.cancel("job-404").unwrap_err().code,
            "unknown_element"
        );
    }

    #[test]
    fn concurrency_is_bounded_and_a_queued_job_can_be_cancelled() {
        let registry = JobRegistry::new(1);
        let release = CancelToken::new();
        let hold = release.clone();
        let first = registry.spawn(JobKind::Proxy, "first", move |_| {
            while !hold.is_cancelled() {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(json!(null))
        });
        // Each job waits for a slot on its own thread and the slots are not handed out in spawn
        // order, so the second job is only guaranteed to queue once the first holds the slot.
        while registry.get(&first).unwrap().state != JobState::Running {
            std::thread::sleep(Duration::from_millis(2));
        }
        let second = registry.spawn(JobKind::Proxy, "second", |_| Ok(json!(null)));
        std::thread::sleep(Duration::from_millis(80));
        assert_eq!(registry.get(&second).unwrap().state, JobState::Queued);
        registry.cancel(&second).unwrap();
        assert_eq!(
            registry
                .wait(&second, Duration::from_secs(5))
                .unwrap()
                .state,
            JobState::Cancelled
        );
        release.cancel();
        assert_eq!(
            registry.wait(&first, Duration::from_secs(5)).unwrap().state,
            JobState::Succeeded
        );
    }

    #[test]
    fn failures_and_panics_become_typed_final_states() {
        let registry = JobRegistry::new(2);
        let failed = registry.spawn(JobKind::Package, "bad", |_| {
            Err(JobError::new("disk_full", "no space").with_details(json!({ "needed": 10 })))
        });
        let info = registry.wait(&failed, Duration::from_secs(5)).unwrap();
        assert_eq!(info.state, JobState::Failed);
        assert_eq!(info.error.as_ref().unwrap().code, "disk_full");
        let command: CommandError = info.error.unwrap().into();
        assert_eq!(command.code, "disk_full");

        let panicked = registry.spawn(JobKind::Package, "panic", |_| panic!("boom"));
        let info = registry.wait(&panicked, Duration::from_secs(5)).unwrap();
        assert_eq!(info.state, JobState::Failed);
        assert_eq!(info.error.unwrap().code, "job_failed");
    }

    #[test]
    fn inline_runs_are_recorded_like_background_jobs() {
        let registry = JobRegistry::new(1);
        let (id, outcome) = registry.run_inline(JobKind::Probe, "probe", |ctx| {
            ctx.stage("probe", 1);
            ctx.advance(1);
            Ok(json!(1))
        });
        assert!(outcome.is_ok());
        assert_eq!(registry.get(&id).unwrap().state, JobState::Succeeded);
        assert_eq!(registry.list().len(), 1);
    }

    fn finished_with_result(registry: &JobRegistry) -> JobId {
        let id = registry.spawn(JobKind::Import, "result", |ctx| {
            ctx.hold_result();
            Ok(json!({ "kept": true }))
        });
        registry.wait(&id, Duration::from_secs(5)).unwrap();
        id
    }

    #[test]
    fn a_result_is_pending_until_it_is_settled_and_prune_keeps_it() {
        let registry = JobRegistry::new(2);
        let plain = registry.spawn(JobKind::Probe, "plain", |_| Ok(json!(1)));
        registry.wait(&plain, Duration::from_secs(5)).unwrap();
        let held = finished_with_result(&registry);
        assert_eq!(
            registry.get(&plain).unwrap().result_state,
            ResultState::None
        );
        assert_eq!(
            registry.get(&held).unwrap().result_state,
            ResultState::Pending
        );
        assert_eq!(
            registry.get(&held).unwrap().to_json()["result_state"],
            "pending"
        );

        registry.prune(0);
        assert!(registry.get(&plain).is_none(), "a plain job is forgotten");
        assert!(registry.get(&held).is_some(), "a held result is not");

        assert!(registry.settle(&held, true));
        assert_eq!(
            registry.get(&held).unwrap().result_state,
            ResultState::Applied
        );
        assert!(
            !registry.settle(&held, false),
            "settling twice changes nothing"
        );
        assert_eq!(
            registry.get(&held).unwrap().result_state,
            ResultState::Applied
        );
        registry.prune(0);
        assert!(registry.get(&held).is_none());
    }

    #[test]
    fn a_failed_or_cancelled_job_holds_nothing() {
        let registry = JobRegistry::new(2);
        let failed = registry.spawn(JobKind::Import, "bad", |ctx| {
            ctx.hold_result();
            Err(JobError::new("io_error", "late failure"))
        });
        registry.wait(&failed, Duration::from_secs(5)).unwrap();
        assert_eq!(
            registry.get(&failed).unwrap().result_state,
            ResultState::None
        );
        registry.prune(0);
        assert!(registry.get(&failed).is_none());
    }

    #[test]
    fn begin_discard_is_one_atomic_typed_gate() {
        let registry = JobRegistry::new(2);
        let held = finished_with_result(&registry);
        let code = |id: &str| registry.begin_discard(id).unwrap_err().code;
        assert_eq!(code("job-404"), "unknown_element");

        let running = registry.spawn(JobKind::Proxy, "running", |ctx| {
            while !ctx.is_cancelled() {
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(JobError::cancelled())
        });
        while registry.get(&running).unwrap().state != JobState::Running {
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(code(&running), "job_running");
        registry.cancel(&running).unwrap();
        registry.wait(&running, Duration::from_secs(5)).unwrap();
        assert_eq!(
            code(&running),
            "job_nothing_to_discard",
            "cancelled: nothing"
        );

        assert_eq!(
            registry.begin_discard(&held).unwrap().result_state,
            ResultState::Discarding
        );
        assert_eq!(code(&held), "job_discarding", "the second caller loses");
        registry.finish_discard(&held);
        assert_eq!(code(&held), "job_discarded");

        let applied = finished_with_result(&registry);
        registry.settle(&applied, true);
        assert_eq!(code(&applied), "job_applied");
        assert_eq!(
            registry.get(&applied).unwrap().result_state,
            ResultState::Applied,
            "a refusal changes nothing"
        );
    }

    #[test]
    fn a_result_being_discarded_cannot_be_applied_and_is_not_pruned() {
        let registry = JobRegistry::new(2);
        let held = finished_with_result(&registry);
        registry.begin_discard(&held).unwrap();
        assert!(!registry.settle(&held, true), "the discard owns it now");
        registry.prune(0);
        assert_eq!(
            registry.get(&held).unwrap().result_state,
            ResultState::Discarding
        );
        registry.finish_discard(&held);
        registry.prune(0);
        assert!(registry.get(&held).is_none());
    }

    #[test]
    fn a_discard_job_is_not_cancellable() {
        let registry = JobRegistry::new(1);
        let release = CancelToken::new();
        let hold = release.clone();
        let id = registry.spawn(JobKind::Discard, "discard", move |_| {
            while !hold.is_cancelled() {
                std::thread::sleep(Duration::from_millis(2));
            }
            Ok(json!(null))
        });
        let error = registry.cancel(&id).unwrap_err();
        assert_eq!(error.code, "job_not_cancellable");
        assert!(!registry.get(&id).unwrap().cancel_requested);
        assert_eq!(registry.cancel_all(), 0);
        release.cancel();
        assert_eq!(
            registry.wait(&id, Duration::from_secs(5)).unwrap().state,
            JobState::Succeeded
        );
    }
}
