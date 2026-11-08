//! The immutable snapshot of the history: what a reader sees, what freezing fixes and what the
//! export writes. Everything here is derived from a copy; nothing mutates the recorder.

use std::collections::VecDeque;
use std::time::Duration;

use serde_json::{Map, Value, json};

use super::{
    Agg, BUCKET, BUCKETS, Context, Counters, Event, MAX_EVENTS, METRICS, Metric, Ring, SCHEMA,
    SCHEMA_VERSION, Segment, WINDOW, bucket_of, build_info, ms, round3,
};

/// What [`Snapshot::capture`] copies from the recorder.
pub(super) struct Capture<'a> {
    pub ring: &'a Ring,
    pub horizon: Duration,
    pub started: Duration,
    pub ended: Option<Duration>,
    pub events: &'a VecDeque<Event>,
    pub segments: &'a VecDeque<Segment>,
    pub dropped_events: u64,
    pub counters: Counters,
    pub context: Option<Context>,
    pub segment: u32,
    pub expected_motion: bool,
    pub open_monitor_ms: Option<f64>,
}

/// The last 60 seconds as of one instant: 240 buckets of 250 ms, the events inside them and the
/// segments they span. The last bucket holds `captured_at`; earlier ones that were never
/// recorded (before the recording began) are empty and are outside `recording_start`.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    /// Fixed by [`Recorder::freeze`](super::Recorder::freeze) (a moving window is not).
    pub frozen: bool,
    /// Where the window ends: `now`, or the instant a stopped recording stopped.
    pub captured_at: Duration,
    /// Index of the first bucket (can be negative while the recording is younger than 60 s).
    pub first_bucket: i64,
    pub recording_start: Duration,
    /// `None` while still recording.
    pub recording_end: Option<Duration>,
    pub buckets: Vec<[Agg; METRICS]>,
    pub events: Vec<Event>,
    pub segments: Vec<Segment>,
    pub dropped_events: u64,
    pub counters: Counters,
    pub context: Option<Context>,
    /// The segment (opaque session id) current at `captured_at`.
    pub segment: u32,
    /// A playback clock was running: silence is a stall, not a rest.
    pub expected_motion: bool,
    /// While frames are expected and none arrived: how long the monitor has been silent.
    pub open_monitor_interval_ms: Option<f64>,
    // Kept for the export's `window` block.
    pub window_start: f64,
    pub window_end: f64,
}

/// Count, extremes, weighted mean and where the maximum was seen, over a whole window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MetricSummary {
    pub count: u64,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    /// Milliseconds on the history's clock where the maximum was sampled.
    pub peak_at_ms: f64,
}

impl MetricSummary {
    fn to_json(self) -> Value {
        json!({
            "count": self.count, "min": round3(self.min), "max": round3(self.max),
            "mean": round3(self.mean), "peak_at_ms": round3(self.peak_at_ms),
        })
    }
}

/// What selecting an instant shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Inspection {
    pub bucket: usize,
    pub start_ms: f64,
    pub end_ms: f64,
    /// The bucket falls inside the recorded range (an empty one outside it was never observed).
    pub observed: bool,
    pub segment: Option<u32>,
    pub metrics: [Agg; METRICS],
    /// Events that overlap the bucket or the longest wait that ended in it.
    pub events: Vec<Event>,
    pub from_ms: f64,
    pub to_ms: f64,
}

impl Inspection {
    pub fn to_json(&self) -> Value {
        let mut metrics = Map::new();
        for metric in Metric::ALL {
            metrics.insert(
                metric.id().to_string(),
                agg_json(&self.metrics[metric.index()]),
            );
        }
        json!({
            "bucket": { "index": self.bucket, "start_ms": round3(self.start_ms),
                "end_ms": round3(self.end_ms) },
            "observed": self.observed,
            "segment": self.segment,
            "metrics": metrics,
            "events": self.events.iter().map(Event::to_json).collect::<Vec<_>>(),
            "correlation": { "from_ms": round3(self.from_ms), "to_ms": round3(self.to_ms),
                "rule": "events inside the bucket or inside the longest wait (interval/latency) that ended in it" },
            "note": "numbers are the bucket's count/min/mean/max; raw samples are not kept",
        })
    }
}

fn agg_json(agg: &Agg) -> Value {
    if agg.is_empty() {
        return Value::Null;
    }
    json!({
        "count": agg.count, "min": round3(agg.min), "mean": agg.mean().map(round3),
        "max": round3(agg.max), "max_at_ms": round3(agg.max_at_us as f64 / 1000.0),
    })
}

impl Snapshot {
    pub(super) fn capture(from: Capture<'_>) -> Self {
        let end = bucket_of(from.horizon) as i64;
        let first = end - (BUCKETS as i64 - 1);
        let mut buckets = Vec::with_capacity(BUCKETS);
        for k in 0..BUCKETS as i64 {
            let index = first + k;
            buckets.push(if index >= 0 {
                from.ring.at(index as u64)
            } else {
                [Agg::default(); METRICS]
            });
        }
        let window_start = first as f64 * ms(BUCKET);
        let window_end = (first + BUCKETS as i64) as f64 * ms(BUCKET);
        let events: Vec<Event> = from
            .events
            .iter()
            .filter(|event| ms(event.at) >= window_start && event.at <= from.horizon)
            .take(MAX_EVENTS)
            .cloned()
            .collect();
        let segments = from
            .segments
            .iter()
            .enumerate()
            .filter(|(position, segment)| {
                let next = from.segments.get(position + 1).map(|next| next.start);
                let end = next.unwrap_or(from.horizon);
                ms(end) > window_start && segment.start <= from.horizon
            })
            .map(|(_, segment)| *segment)
            .collect();
        Self {
            frozen: false,
            captured_at: from.horizon,
            first_bucket: first,
            recording_start: from.started,
            recording_end: from.ended,
            buckets,
            events,
            segments,
            dropped_events: from.dropped_events,
            counters: from.counters,
            context: from.context,
            segment: from.segment,
            expected_motion: from.expected_motion,
            open_monitor_interval_ms: from.open_monitor_ms,
            window_start,
            window_end,
        }
    }

    pub fn bucket_start_ms(&self, bucket: usize) -> f64 {
        (self.first_bucket + bucket as i64) as f64 * ms(BUCKET)
    }

    /// The bucket `at` falls in, `None` outside the window.
    pub fn index_at(&self, at: Duration) -> Option<usize> {
        // Checked all the way: an absurd instant must not wrap into a valid bucket (a recording
        // younger than the window has a negative first bucket) nor overflow the subtraction.
        let index = i64::try_from(bucket_of(at))
            .ok()?
            .checked_sub(self.first_bucket)?;
        usize::try_from(index).ok().filter(|index| *index < BUCKETS)
    }

    /// The series of one metric, one entry per bucket (`None`: nothing was observed).
    pub fn series(&self, metric: Metric) -> impl Iterator<Item = Option<&Agg>> {
        self.buckets
            .iter()
            .map(move |aggs| Some(&aggs[metric.index()]).filter(|agg| !agg.is_empty()))
    }

    /// Whether a bucket lies in the recorded range.
    pub fn observed(&self, bucket: usize) -> bool {
        let start = self.bucket_start_ms(bucket);
        let end = start + ms(BUCKET);
        end > ms(self.recording_start) && self.recording_end.is_none_or(|ended| start < ms(ended))
    }

    /// Count-weighted summary over the window; `None` when no sample exists.
    pub fn summary(&self, metric: Metric) -> Option<MetricSummary> {
        let mut total = Agg::default();
        for aggs in &self.buckets {
            total.merge(&aggs[metric.index()]);
        }
        (!total.is_empty()).then(|| MetricSummary {
            count: u64::from(total.count),
            min: total.min,
            max: total.max,
            mean: total.sum / f64::from(total.count),
            peak_at_ms: total.max_at_us as f64 / 1000.0,
        })
    }

    /// The segment active at the end of `bucket`.
    fn segment_at(&self, bucket: usize) -> Option<u32> {
        let end = self.bucket_start_ms(bucket) + ms(BUCKET);
        self.segments
            .iter()
            .rev()
            .find(|segment| ms(segment.start) < end)
            .map(|segment| segment.id)
    }

    /// Numbers, extremes and correlated events at `at`; `None` outside the window.
    pub fn inspect(&self, at: Duration) -> Option<Inspection> {
        let bucket = self.index_at(at)?;
        let start_ms = self.bucket_start_ms(bucket);
        let end_ms = start_ms + ms(BUCKET);
        let metrics = self.buckets[bucket];
        // A wait is sampled when it *ends*: the event that explains a long interval or latency
        // happened before its sample, so look back as far as the longest wait of the bucket.
        let from_ms = Metric::ALL
            .into_iter()
            .filter(|metric| metric.is_wait())
            .map(|metric| &metrics[metric.index()])
            .filter(|agg| !agg.is_empty())
            .map(|agg| (agg.max_at_us as f64 / 1000.0 - agg.max).max(self.window_start))
            .fold(start_ms, f64::min);
        let events = self
            .events
            .iter()
            .filter(|event| ms(event.at) >= from_ms && ms(event.at) < end_ms)
            .cloned()
            .collect();
        Some(Inspection {
            bucket,
            start_ms,
            end_ms,
            observed: self.observed(bucket),
            segment: self.segment_at(bucket),
            metrics,
            events,
            from_ms,
            to_ms: end_ms,
        })
    }

    fn window_json(&self) -> Value {
        json!({
            "start_ms": round3(self.window_start), "end_ms": round3(self.window_end),
            "duration_ms": ms(WINDOW), "bucket_ms": ms(BUCKET), "buckets": BUCKETS,
            "captured_at_ms": round3(ms(self.captured_at)),
            "unobserved_before_ms": round3(ms(self.recording_start)),
        })
    }

    fn peaks_json(&self) -> Value {
        let mut peaks = Map::new();
        for metric in Metric::ALL {
            peaks.insert(
                metric.id().to_string(),
                self.summary(metric)
                    .map_or(Value::Null, MetricSummary::to_json),
            );
        }
        Value::Object(peaks)
    }

    fn state_json(&self) -> Value {
        json!({
            "expected_motion": self.expected_motion,
            "monitor_open_interval_ms": self.open_monitor_interval_ms.map(round3),
            "segment": self.segment,
        })
    }

    fn counters_json(&self) -> Value {
        json!({
            "stale_responses_dropped": self.counters.stale_responses,
            "pictures_without_admitted_request": self.counters.unmatched_images,
            "requests_forgotten": self.counters.forgotten_requests,
            "late_samples_dropped": self.counters.late_samples,
            "memory_readings_unavailable": self.counters.memory_unavailable,
        })
    }

    /// The small form for `get`: flags, window, peaks per metric, newest events.
    pub fn compact_json(&self) -> Value {
        let newest: Vec<Value> = self
            .events
            .iter()
            .rev()
            .take(30)
            .rev()
            .map(Event::to_json)
            .collect();
        json!({
            "frozen": self.frozen,
            "window": self.window_json(),
            "recording": {
                "started_at_ms": round3(ms(self.recording_start)),
                "ended_at_ms": self.recording_end.map(|at| round3(ms(at))),
            },
            "peaks": self.peaks_json(),
            "events": { "held": self.events.len(), "capacity": MAX_EVENTS,
                "dropped": self.dropped_events, "newest": newest },
            "segments": self.segments.iter().map(segment_json).collect::<Vec<_>>(),
            "state": self.state_json(),
            "counters": self.counters_json(),
            "context": self.context.as_ref().map(|context| context.to_json(self.segment)),
        })
    }

    /// The exported document (schema v1): units, sources and limits, monotonic origin, recording
    /// range, non-sensitive context, buckets (count/min/mean/max per metric) and events. No
    /// project text, absolute path, credential or media content is ever put here.
    pub fn to_document(&self) -> Value {
        let mut metrics = Map::new();
        let mut series = Map::new();
        for metric in Metric::ALL {
            metrics.insert(
                metric.id().to_string(),
                json!({
                    "unit": metric.unit(), "sampling": match metric.kind() {
                        super::Kind::Event => "per occurrence",
                        super::Kind::Gauge => "gauge, at most once per second",
                    },
                    "scale_group": metric.group(), "source": metric.source(),
                }),
            );
            let column = |read: fn(&Agg) -> Value| -> Vec<Value> {
                self.buckets
                    .iter()
                    .map(|aggs| {
                        let agg = &aggs[metric.index()];
                        if agg.is_empty() {
                            Value::Null
                        } else {
                            read(agg)
                        }
                    })
                    .collect()
            };
            series.insert(
                metric.id().to_string(),
                json!({
                    "count": column(|agg| json!(agg.count)),
                    "min": column(|agg| json!(round3(agg.min))),
                    "mean": column(|agg| json!(agg.mean().map(round3))),
                    "max": column(|agg| json!(round3(agg.max))),
                }),
            );
        }
        json!({
            "schema": SCHEMA,
            "schema_version": SCHEMA_VERSION,
            "build": build_info(),
            "monotonic_origin": {
                "kind": "process monotonic clock (the executor's Instant) at the editor window creation",
                "unit": "ms",
                "note": "every *_ms time in this file is milliseconds since that origin; the page's JavaScript clock is never subtracted from it",
            },
            "recording": {
                "state": if self.recording_end.is_some() { "stopped" } else { "recording" },
                "started_at_ms": round3(ms(self.recording_start)),
                "ended_at_ms": self.recording_end.map(|at| round3(ms(at))),
                "frozen": self.frozen,
                "captured_at_ms": round3(ms(self.captured_at)),
            },
            "window": self.window_json(),
            "state": self.state_json(),
            "context": self.context.as_ref().map(|context| context.to_json(self.segment)),
            "metrics": metrics,
            "series": series,
            "summary": self.peaks_json(),
            "events": self.events.iter().map(Event::to_json).collect::<Vec<_>>(),
            "events_capacity": MAX_EVENTS,
            "events_dropped": self.dropped_events,
            "segments": self.segments.iter().map(segment_json).collect::<Vec<_>>(),
            "counters": self.counters_json(),
            "limits": [
                "absent (null) means nothing was observed, never zero; a still picture is a rest, not a stall",
                "ui_interval_ms keeps a paint gap of any length while a playback clock runs (a stall); the window being hidden or occluded by the system is not told apart from one",
                "monitor/ui rates are pictures accepted by the host and frames GPUI painted: not GPU completion, compositor presentation or seek-to-photon",
                "render_ms is CPU time until submission; latency_ms is host request admission to acceptance of the picture of the same request id",
                "rss_bytes is the editor process only (no WebKit WebContent/GPU); texture_bytes is the runtime's estimate; neither is GPU memory",
                "buckets keep count/min/mean/max; there are no percentiles and raw samples are not kept",
                "no project text, absolute paths, credentials or media content are exported",
                "live measurement exists only in the desktop window; the headless CLI has no presentation clock",
            ],
        })
    }
}

fn segment_json(segment: &Segment) -> Value {
    json!({ "id": segment.id, "start_ms": round3(ms(segment.start)), "kind": segment.kind.id() })
}
