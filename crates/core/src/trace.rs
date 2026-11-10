//! Opt-in timing of the host side of a preview frame.
//!
//! The runtime reports what it spends drawing (`frame.done.timings`); this module measures the
//! part around it: planning, preparing and serializing the request, handing it to the transport,
//! the time in transit, and accepting the answer. Together they say where the indicator's
//! milliseconds go instead of attributing the whole of it to "render".
//!
//! WHY a shared handle and not a field of the session: the desktop host also times work the
//! session never sees (the plan it builds before asking, the script evaluation, serving assets),
//! and the protocol handler lives detached from the host. One cloneable handle lets all of them
//! record into the same place.
//!
//! Cost when off: every method begins with one relaxed atomic load and returns; no clock is read
//! ([`TraceHandle::start`] gives `None`). Cost when on: a clock read per span and an uncontended
//! mutex lock per record, a few hundred nanoseconds per frame in all. What is measured, and what
//! is not, is part of the contract: see `docs/features/viewport.md` ("Host spans").
//!
//! Clocks: every span here is measured with the host's monotonic clock. The runtime's own
//! timings use the page clock (1 ms resolution in WebKit); the two are never subtracted from
//! each other except `transit_ms`, which is a difference of a host interval and a page
//! interval and is therefore labelled an estimate.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::backend::{FrameResult, RequestId};

/// Samples kept per span for percentiles; older ones are overwritten (the ring is the window).
const SAMPLE_WINDOW: usize = 512;
/// Frames kept for [`TraceHandle::snapshot`].
const FRAME_WINDOW: usize = 256;
/// Events the correlation log keeps. A play of the real project writes ~3 000 (ticks, syncs, prefetches, asset requests,
/// runtime spans): the window is several times that, and a reader that polls with a cursor sees `lost` if it ever falls
/// behind instead of a silent hole.
pub const EVENT_WINDOW: usize = 16_384;

/// What the backend measured while handing one request over.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SubmitSpans {
    /// Building the request message (plan to JSON value).
    pub serialize: Duration,
    /// Giving it to the transport. For the WebView this is the text encoding and queueing; the
    /// script evaluation happens later, in the host, and is its own span (`evaluate`).
    pub dispatch: Duration,
}

/// One frame's host-side breakdown, in milliseconds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FrameTrace {
    pub id: RequestId,
    pub revision: u64,
    pub interactive: bool,
    /// Session work from the request to having it ready to send (asset checks and loads).
    pub prepare_ms: f64,
    pub serialize_ms: f64,
    pub dispatch_ms: f64,
    /// Handing over to the answer arriving, on the host clock.
    pub submit_to_done_ms: f64,
    /// `submit_to_done_ms` minus the runtime's `total_ms`: queueing, evaluation, the return trip
    /// and the host loop picking the answer up. An estimate, see the module comment.
    pub transit_ms: f64,
    pub runtime_total_ms: f64,
    pub runtime_render_ms: f64,
    /// The session handling the answer (state update, stale check).
    pub accept_ms: f64,
    /// Host clock, milliseconds since the trace was created: when the request was handed over and
    /// when its answer was accepted. These are what a proof reads to get the real spacing of the
    /// pictures instead of sampling the editor from outside (sampling adds its own load and can
    /// only see a lower bound of the rate).
    pub submitted_ms: f64,
    pub done_ms: f64,
    /// The sequence frame the accepted picture shows (`None` for a source preview).
    pub shown_frame: Option<i64>,
    /// Page clock: `frame.done` to the next animation frame. Only when the host opted in.
    pub presented_after_done_ms: Option<f64>,
    /// Host clock: handing over to the `frame.presented` message. Only when the host opted in.
    pub submit_to_presented_ms: Option<f64>,
    /// Runtime page clock, durations only (never subtracted from a host stamp): what the frame spent preparing its
    /// layers (decode/upload of what it needed), drawing and flushing. CPU time of the page, not GPU time.
    pub runtime_prepare_ms: f64,
    pub runtime_draw_ms: f64,
    pub runtime_flush_ms: f64,
}

impl FrameTrace {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id, "revision": self.revision, "interactive": self.interactive,
            "prepare_ms": round(self.prepare_ms), "serialize_ms": round(self.serialize_ms),
            "dispatch_ms": round(self.dispatch_ms),
            "submit_to_done_ms": round(self.submit_to_done_ms),
            "transit_ms": round(self.transit_ms),
            "runtime_total_ms": round(self.runtime_total_ms),
            "runtime_render_ms": round(self.runtime_render_ms),
            "accept_ms": round(self.accept_ms),
            "submitted_ms": round(self.submitted_ms), "done_ms": round(self.done_ms),
            "shown_frame": self.shown_frame,
            "presented_after_done_ms": self.presented_after_done_ms.map(round),
            "submit_to_presented_ms": self.submit_to_presented_ms.map(round),
            "runtime_prepare_ms": round(self.runtime_prepare_ms),
            "runtime_draw_ms": round(self.runtime_draw_ms),
            "runtime_flush_ms": round(self.runtime_flush_ms),
        })
    }
}

fn round(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

#[derive(Debug, Default)]
struct Series {
    window: VecDeque<f64>,
    count: u64,
    total: f64,
    max: f64,
}

impl Series {
    fn push(&mut self, value: f64) {
        if self.window.len() == SAMPLE_WINDOW {
            self.window.pop_front();
        }
        self.window.push_back(value);
        self.count += 1;
        self.total += value;
        self.max = self.max.max(value);
    }

    fn to_json(&self) -> Value {
        let mut sorted: Vec<f64> = self.window.iter().copied().collect();
        sorted.sort_by(f64::total_cmp);
        let at = |share: f64| {
            sorted
                .get(((share * sorted.len() as f64) as usize).min(sorted.len().saturating_sub(1)))
                .copied()
                .unwrap_or(0.0)
        };
        let window_mean = if sorted.is_empty() {
            0.0
        } else {
            sorted.iter().sum::<f64>() / sorted.len() as f64
        };
        json!({
            "n": self.count, "window": sorted.len(),
            "mean": round(window_mean), "p50": round(at(0.5)), "p95": round(at(0.95)),
            "p99": round(at(0.99)), "max": round(self.max),
        })
    }
}

#[derive(Debug)]
struct Submitted {
    at: Instant,
    spans: SubmitSpans,
    prepare_ms: f64,
    revision: u64,
    interactive: bool,
}

#[derive(Debug, Default)]
struct Inner {
    spans: BTreeMap<&'static str, Series>,
    frames: VecDeque<FrameTrace>,
    /// Frames handed over and not yet answered. Bounded by the number of requests the session
    /// allows in flight (one), plus the ones it cancelled; stale ones are removed on answer.
    open: BTreeMap<RequestId, Submitted>,
    frames_total: u64,
    dropped: u64,
    /// Plan time noted by the host for the request it is about to make.
    plan_ms: Option<f64>,
}

impl Inner {
    fn record(&mut self, name: &'static str, value: f64) {
        self.spans.entry(name).or_default().push(value);
    }
}

/// One entry of the correlation log: what happened, when (host monotonic clock, milliseconds since the trace was
/// created) and the facts of that kind. Durations inside `fields` are local to their own clock: host spans use the
/// host's, runtime spans the page's, and the two are never subtracted from each other.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceEvent {
    pub seq: u64,
    pub t_ms: f64,
    pub kind: &'static str,
    pub fields: Value,
}

#[derive(Debug, Default)]
struct EventLog {
    ring: VecDeque<TraceEvent>,
    /// Sequence number of the next event; the first is 1, so a cursor of 0 means "from the beginning".
    next: u64,
}

/// Cloneable, thread-safe recorder. Disabled by default.
#[derive(Debug, Clone)]
pub struct TraceHandle {
    enabled: Arc<AtomicBool>,
    inner: Arc<Mutex<Inner>>,
    /// Its own lock: asset workers and the UI thread both write events and must not queue behind the frame table.
    events: Arc<Mutex<EventLog>>,
    epoch: Instant,
}

impl Default for TraceHandle {
    fn default() -> Self {
        Self {
            enabled: Arc::default(),
            inner: Arc::default(),
            events: Arc::default(),
            epoch: Instant::now(),
        }
    }
}

impl TraceHandle {
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A poisoned lock only means a panic elsewhere while recording numbers.
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Host monotonic milliseconds since this trace was created: the stamp every event carries.
    pub fn now_ms(&self) -> f64 {
        ms(self.epoch.elapsed())
    }

    /// Host monotonic milliseconds at `at` (an instant taken earlier).
    pub fn ms_at(&self, at: Instant) -> f64 {
        ms(at.saturating_duration_since(self.epoch))
    }

    /// Appends one event to the bounded correlation log. `fields` is built only when tracing is on, so an
    /// untraced run pays one relaxed load. No flush and no I/O: the log is read back with
    /// [`events_since`](Self::events_since).
    pub fn event(&self, kind: &'static str, fields: impl FnOnce() -> Value) {
        if !self.enabled() {
            return;
        }
        let event_fields = fields();
        let t_ms = self.now_ms();
        let mut log = self
            .events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        log.next += 1;
        let seq = log.next;
        if log.ring.len() == EVENT_WINDOW {
            log.ring.pop_front();
        }
        log.ring.push_back(TraceEvent {
            seq,
            t_ms,
            kind,
            fields: event_fields,
        });
    }

    /// The events after cursor `since` (0 reads from the oldest kept), at most `limit`, as
    /// `{ next, lost, events }`: `next` is the cursor for the following read and `lost` counts events that left the
    /// window before this read (a poller that sees it above 0 invalidates the run instead of trusting a hole).
    pub fn events_since(&self, since: u64, limit: usize) -> Value {
        let log = self
            .events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let oldest = log.ring.front().map_or(log.next + 1, |event| event.seq);
        let lost = oldest.saturating_sub(since + 1);
        let items: Vec<Value> = log
            .ring
            .iter()
            .filter(|event| event.seq > since)
            .take(limit)
            .map(|event| {
                json!({ "seq": event.seq, "t_ms": round(event.t_ms), "kind": event.kind,
                    "fields": event.fields })
            })
            .collect();
        let next = items
            .last()
            .and_then(|item| item["seq"].as_u64())
            .unwrap_or(since.max(oldest.saturating_sub(1)));
        json!({
            "enabled": self.enabled(),
            "now_ms": round(self.now_ms()),
            "next": next,
            "lost": lost,
            "window": EVENT_WINDOW,
            "events": items,
        })
    }

    /// Starts a span; `None` when tracing is off, so callers pay no clock read.
    pub fn start(&self) -> Option<Instant> {
        self.enabled().then(Instant::now)
    }

    /// Ends a span started by [`start`](Self::start) and records it under `name`.
    pub fn span(&self, name: &'static str, started: Option<Instant>) {
        let Some(started) = started else {
            return;
        };
        self.record(name, started.elapsed());
    }

    pub fn record(&self, name: &'static str, elapsed: Duration) {
        if !self.enabled() {
            return;
        }
        self.lock().record(name, ms(elapsed));
    }

    /// The host planned the frame it is about to request in `elapsed` (state to render plan).
    pub fn note_plan(&self, elapsed: Duration) {
        if !self.enabled() {
            return;
        }
        let mut inner = self.lock();
        inner.record("plan", ms(elapsed));
        inner.plan_ms = Some(ms(elapsed));
    }

    /// A request was handed to the backend. `prepare` is the session work before it.
    pub fn frame_submitted(
        &self,
        id: RequestId,
        revision: u64,
        interactive: bool,
        prepare: Duration,
        spans: SubmitSpans,
        at: Instant,
    ) {
        if !self.enabled() {
            return;
        }
        let mut inner = self.lock();
        inner.record("prepare", ms(prepare));
        inner.record("serialize", ms(spans.serialize));
        inner.record("dispatch", ms(spans.dispatch));
        // A cancelled request never answers (or answers stale): keep the table small.
        while inner.open.len() >= 16 {
            inner.open.pop_first();
        }
        inner.open.insert(
            id,
            Submitted {
                at,
                spans,
                prepare_ms: ms(prepare),
                revision,
                interactive,
            },
        );
    }

    /// The answer to an accepted request arrived; `accept` is the time the session spent on it.
    pub fn frame_done(
        &self,
        frame: &FrameResult,
        shown_frame: Option<i64>,
        received: Instant,
        accept: Duration,
    ) {
        if !self.enabled() {
            return;
        }
        let mut inner = self.lock();
        let Some(submitted) = inner.open.remove(&frame.id) else {
            return;
        };
        let submit_to_done = ms(received.saturating_duration_since(submitted.at));
        let trace = FrameTrace {
            id: frame.id,
            revision: submitted.revision,
            interactive: submitted.interactive,
            prepare_ms: submitted.prepare_ms,
            serialize_ms: ms(submitted.spans.serialize),
            dispatch_ms: ms(submitted.spans.dispatch),
            submit_to_done_ms: submit_to_done,
            transit_ms: (submit_to_done - frame.total_ms).max(0.0),
            runtime_total_ms: frame.total_ms,
            runtime_render_ms: frame.render_ms,
            accept_ms: ms(accept),
            submitted_ms: ms(submitted.at.saturating_duration_since(self.epoch)),
            done_ms: ms(received.saturating_duration_since(self.epoch)),
            shown_frame,
            presented_after_done_ms: None,
            submit_to_presented_ms: None,
            runtime_prepare_ms: frame.detail.prepare_ms,
            runtime_draw_ms: frame.detail.draw_ms,
            runtime_flush_ms: frame.detail.flush_ms,
        };
        inner.record("submit_to_done", trace.submit_to_done_ms);
        inner.record("transit", trace.transit_ms);
        inner.record("runtime_total", trace.runtime_total_ms);
        inner.record("runtime_render", trace.runtime_render_ms);
        inner.record("accept", trace.accept_ms);
        inner.frames_total += 1;
        if inner.frames.len() == FRAME_WINDOW {
            inner.frames.pop_front();
        }
        inner.frames.push_back(trace);
    }

    /// `frame.presented` arrived for `id` (only when the host asked for it in `hello`). The
    /// frame must already be accepted: `frame.presented` follows `frame.done` on the bridge.
    pub fn frame_presented(&self, id: RequestId, after_done_ms: f64) {
        if !self.enabled() {
            return;
        }
        let mut inner = self.lock();
        inner.record("presented_after_done", after_done_ms);
        let Some(trace) = inner.frames.iter_mut().rev().find(|trace| trace.id == id) else {
            return;
        };
        trace.presented_after_done_ms = Some(after_done_ms);
        // Derived, not observed: host time to the answer plus the page's own wait for the next
        // animation frame, so no clock of the page is ever subtracted from the host's.
        let host_total = trace.submit_to_done_ms + after_done_ms;
        trace.submit_to_presented_ms = Some(host_total);
        inner.record("submit_to_presented", host_total);
    }

    /// A request ended without an accepted picture (cancelled, stale answer, failure).
    pub fn frame_dropped(&self, id: RequestId) {
        if !self.enabled() {
            return;
        }
        let mut inner = self.lock();
        inner.open.remove(&id);
        inner.dropped += 1;
    }

    pub fn reset(&self) {
        *self.lock() = Inner::default();
        *self
            .events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = EventLog::default();
    }

    /// Summary of every span (percentiles over the last samples) and the last `last` frames.
    pub fn snapshot(&self, last: usize) -> Value {
        let inner = self.lock();
        let spans: serde_json::Map<String, Value> = inner
            .spans
            .iter()
            .map(|(name, series)| ((*name).to_string(), series.to_json()))
            .collect();
        let skip = inner.frames.len().saturating_sub(last);
        json!({
            "enabled": self.enabled(),
            // The trace's own clock now, to place a window of interest on the `*_ms` stamps.
            "now_ms": round(ms(self.epoch.elapsed())),
            "frames_total": inner.frames_total,
            "dropped": inner.dropped,
            "window": { "samples": SAMPLE_WINDOW, "frames": FRAME_WINDOW },
            "spans": spans,
            "recent": inner.frames.iter().skip(skip).map(FrameTrace::to_json).collect::<Vec<_>>(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(id: RequestId, total_ms: f64) -> FrameResult {
        FrameResult {
            id,
            revision: 1,
            width: 2,
            height: 2,
            hash: String::new(),
            mean: [0.0; 4],
            probes: Vec::new(),
            png: None,
            render_ms: total_ms / 2.0,
            total_ms,
            detail: Default::default(),
        }
    }

    #[test]
    fn off_records_nothing_and_reads_no_clock() {
        let trace = TraceHandle::default();
        assert!(trace.start().is_none());
        trace.record("x", Duration::from_millis(5));
        trace.note_plan(Duration::from_millis(1));
        trace.frame_dropped(1);
        let snapshot = trace.snapshot(10);
        assert_eq!(snapshot["spans"], json!({}));
        assert_eq!(snapshot["dropped"], 0);
    }

    #[test]
    fn a_frame_gets_its_spans_and_transit_is_what_the_runtime_did_not_spend() {
        let trace = TraceHandle::default();
        trace.set_enabled(true);
        let submitted = Instant::now();
        trace.frame_submitted(
            4,
            9,
            true,
            Duration::from_micros(300),
            SubmitSpans {
                serialize: Duration::from_micros(200),
                dispatch: Duration::from_micros(100),
            },
            submitted,
        );
        let mut done = frame(4, 2.0);
        done.detail.prepare_ms = 0.5;
        done.detail.draw_ms = 0.25;
        done.detail.flush_ms = 0.125;
        trace.frame_done(
            &done,
            Some(12),
            submitted + Duration::from_millis(5),
            Duration::from_micros(50),
        );
        trace.frame_presented(4, 8.0);
        let snapshot = trace.snapshot(1);
        let recent = &snapshot["recent"][0];
        assert_eq!(recent["id"], 4);
        assert_eq!(recent["transit_ms"], 3.0);
        assert_eq!(recent["prepare_ms"], 0.3);
        assert_eq!(recent["serialize_ms"], 0.2);
        assert_eq!(recent["submit_to_presented_ms"], 13.0);
        assert_eq!(recent["shown_frame"], 12);
        assert_eq!(recent["runtime_prepare_ms"], 0.5);
        assert_eq!(recent["runtime_draw_ms"], 0.25);
        assert_eq!(recent["runtime_flush_ms"], 0.125);
        let gap = recent["done_ms"].as_f64().unwrap() - recent["submitted_ms"].as_f64().unwrap();
        assert!((gap - 5.0).abs() < 0.01, "stamps share one clock: {gap}");
        assert_eq!(snapshot["spans"]["transit"]["n"], 1);
        assert_eq!(snapshot["frames_total"], 1);
    }

    #[test]
    fn an_answer_without_a_hand_over_is_ignored_and_dropped_ones_are_counted() {
        let trace = TraceHandle::default();
        trace.set_enabled(true);
        trace.frame_done(&frame(1, 1.0), None, Instant::now(), Duration::ZERO);
        assert_eq!(trace.snapshot(5)["frames_total"], 0);
        trace.frame_submitted(
            2,
            1,
            false,
            Duration::ZERO,
            SubmitSpans::default(),
            Instant::now(),
        );
        trace.frame_dropped(2);
        trace.frame_done(&frame(2, 1.0), None, Instant::now(), Duration::ZERO);
        let snapshot = trace.snapshot(5);
        assert_eq!(snapshot["dropped"], 1);
        assert_eq!(snapshot["frames_total"], 0);
    }

    #[test]
    fn events_are_off_by_default_ordered_and_read_with_a_cursor() {
        let trace = TraceHandle::default();
        let built = std::cell::Cell::new(false);
        trace.event("tick", || {
            built.set(true);
            json!({})
        });
        assert!(!built.get(), "an untraced event builds nothing");
        assert_eq!(trace.events_since(0, 10)["events"], json!([]));
        trace.set_enabled(true);
        for index in 0..5 {
            trace.event("tick", || json!({ "n": index }));
        }
        let first = trace.events_since(0, 3);
        assert_eq!(first["events"].as_array().unwrap().len(), 3);
        assert_eq!(first["next"], 3);
        assert_eq!(first["lost"], 0);
        let rest = trace.events_since(first["next"].as_u64().unwrap(), 10);
        let seqs: Vec<u64> = rest["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["seq"].as_u64().unwrap())
            .collect();
        assert_eq!(seqs, vec![4, 5]);
        // Nothing new: the cursor stays where it was.
        assert_eq!(trace.events_since(5, 10)["next"], 5);
    }

    #[test]
    fn a_reader_that_falls_behind_the_window_is_told_how_much_it_lost() {
        let trace = TraceHandle::default();
        trace.set_enabled(true);
        for _ in 0..(EVENT_WINDOW + 10) {
            trace.event("x", || json!({}));
        }
        let read = trace.events_since(0, EVENT_WINDOW + 10);
        assert_eq!(read["lost"], 10);
        assert_eq!(read["events"].as_array().unwrap().len(), EVENT_WINDOW);
        assert_eq!(read["next"], (EVENT_WINDOW + 10) as u64);
        // A reader that kept up loses nothing.
        assert_eq!(
            trace.events_since(read["next"].as_u64().unwrap(), 5)["lost"],
            0
        );
    }

    #[test]
    fn windows_are_bounded_and_percentiles_come_from_the_window() {
        let trace = TraceHandle::default();
        trace.set_enabled(true);
        for value in 1..=(SAMPLE_WINDOW as u64 + 100) {
            trace.record("span", Duration::from_millis(value));
        }
        let span = &trace.snapshot(0)["spans"]["span"];
        assert_eq!(span["window"], SAMPLE_WINDOW);
        assert_eq!(span["n"], SAMPLE_WINDOW as u64 + 100);
        // The oldest 100 samples left the window, so the median is above the all-time median.
        assert!(span["p50"].as_f64().unwrap() > 300.0);
        trace.reset();
        assert_eq!(trace.snapshot(0)["spans"], json!({}));
    }
}
