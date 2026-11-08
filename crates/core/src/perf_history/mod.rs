//! Rolling performance history (F049, BL145.2): the last 60 seconds of the debug meter's feeds,
//! kept in a fixed ring of 250 ms buckets, with the events that happened next to them.
//!
//! No GPUI, no thread, no real clock: every call takes the time it is told about (a `Duration`
//! since a monotonic origin the adapter chooses), so the rules are testable with a fake clock and
//! the window, the library and a headless caller share them. The history is a **recorder**:
//! off by default, local, bounded in memory, never written to the project, to undo or to
//! autosave. Turning it on after a problem does not recover what came before.
//!
//! What it keeps (see [`Metric`]): per 250 ms bucket and per metric a count, minimum, sum and
//! maximum, so a spike survives the bucketing and the mean is weighted by count. There are no
//! percentiles. Events live in a separate ring (1024) that says how many it discarded.
//!
//! Honesty rules, each covered by a test:
//!
//! - A bucket with no sample is *absent* (`None`/`null`), never zero and never the last value.
//!   A still picture is a rest, not a stall: a gap longer than [`REST_AFTER`] records no interval
//!   unless frames were expected (playback). A failed or lost source does not carry its last
//!   value forward.
//! - Eviction and gaps follow elapsed time: [`Recorder::snapshot`] windows on the `now` it is
//!   given, so old data leaves even when no new frame arrives.
//! - Playback losses are the *delta* of the cumulative counter per playback run; a seek or a
//!   restart opens a new run with a fresh anchor, so it is never added twice.
//! - Latency is host request admission to the acceptance of the picture with the same request
//!   id, on one clock. `render_ms` stays CPU time until submission, separate from it. None of
//!   these is GPU completion, compositor presentation or seek-to-photon.
//! - A session change (project replaced, scene changed, viewport reloaded) starts a new segment:
//!   no interval, loss delta or latency crosses it, and answers to the old session are dropped.
//!
//! Heavy work is bounded: a snapshot copies 240 × 9 small aggregates and at most 1024 events.

mod export;
mod snapshot;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use serde_json::{Value, json};

use crate::jobs::JobState;
use crate::state::CommandError;

pub use export::{HistoryError, PreparedExport, build_info, write_export};
pub use snapshot::{Inspection, MetricSummary, Snapshot};

/// Automation target of the history.
pub const TARGET: &str = "perf.history";
/// Actions of `invoke perf.history <action>`; fields of `set` are in `commands`.
pub const ACTIONS: [&str; 10] = [
    "record_start",
    "record_stop",
    "show",
    "hide",
    "freeze",
    "resume",
    "clear",
    "snapshot",
    "clear_selection",
    "export",
];
/// Length of the rolling window.
pub const WINDOW: Duration = Duration::from_secs(60);
/// One bucket of the ring.
pub const BUCKET: Duration = Duration::from_millis(250);
/// Buckets in the window: 60 s / 250 ms.
pub const BUCKETS: usize = 240;
/// Events kept; the oldest is discarded (and counted) past this.
pub const MAX_EVENTS: usize = 1024;
/// Segments kept.
pub const MAX_SEGMENTS: usize = 64;
/// Requests admitted and still waiting for their picture; the oldest is forgotten past this.
pub const MAX_PENDING: usize = 32;
/// No frame for this long with nothing expected: a rest, not an interval.
pub const REST_AFTER: Duration = Duration::from_millis(1000);
/// Gauges (memory) are sampled at most this often.
pub const GAUGE_EVERY: Duration = Duration::from_millis(1000);
/// Seeks closer than this to the **first** seek of a group are one event with a repeat count
/// (the event keeps the group's start time and the newest frame), so a long scrub reads as a
/// series of at most 250 ms events rather than one that hides its length.
pub const SEEK_COALESCE: Duration = Duration::from_millis(250);
/// Detail of `recording_started` / `recording_cleared` when a playback clock was already running:
/// the context the recording began in, not a play that was observed.
pub const PLAYING_CONTEXT: &str = "playing";
/// Version of the exported schema.
pub const SCHEMA_VERSION: u32 = 1;
/// Name of the exported schema.
pub const SCHEMA: &str = "tessel.perf-history";

// ---------------------------------------------------------------------------- metrics

/// One series of the history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Metric {
    /// Frames the window painted: one sample of `1` per effective paint.
    UiFrames,
    /// Gap between two painted frames of one run, in milliseconds.
    UiIntervalMs,
    /// Pictures the host accepted: one sample of `1` per new image.
    MonitorFrames,
    /// Gap between two accepted pictures of one run, in milliseconds.
    MonitorIntervalMs,
    /// `render_ms` of an accepted picture: CPU time until the draw was submitted.
    RenderMs,
    /// Host request admission to acceptance of the picture of the same request, in milliseconds.
    LatencyMs,
    /// Playback frames lost since the previous observation of the same run.
    LostFrames,
    /// Resident set of the editor process, in bytes (gauge).
    RssBytes,
    /// The runtime's estimate of retained texture bytes (gauge).
    TextureBytes,
}

/// How many metrics there are.
pub const METRICS: usize = 9;

/// How a metric is sampled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// One sample per observed occurrence.
    Event,
    /// A reading of a level, at most once per [`GAUGE_EVERY`].
    Gauge,
}

impl Metric {
    pub const ALL: [Metric; METRICS] = [
        Self::UiFrames,
        Self::UiIntervalMs,
        Self::MonitorFrames,
        Self::MonitorIntervalMs,
        Self::RenderMs,
        Self::LatencyMs,
        Self::LostFrames,
        Self::RssBytes,
        Self::TextureBytes,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::UiFrames => "ui_frames",
            Self::UiIntervalMs => "ui_interval_ms",
            Self::MonitorFrames => "monitor_frames",
            Self::MonitorIntervalMs => "monitor_interval_ms",
            Self::RenderMs => "render_ms",
            Self::LatencyMs => "latency_ms",
            Self::LostFrames => "lost_frames",
            Self::RssBytes => "rss_bytes",
            Self::TextureBytes => "texture_bytes",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|metric| metric.id() == id)
    }

    /// Unit of the values; series of different units never share an axis.
    pub fn unit(self) -> &'static str {
        match self {
            Self::UiFrames | Self::MonitorFrames => "frames",
            Self::UiIntervalMs | Self::MonitorIntervalMs | Self::RenderMs | Self::LatencyMs => "ms",
            Self::LostFrames => "frames",
            Self::RssBytes | Self::TextureBytes => "bytes",
        }
    }

    pub fn kind(self) -> Kind {
        match self {
            Self::RssBytes | Self::TextureBytes => Kind::Gauge,
            _ => Kind::Event,
        }
    }

    /// What a sample is, and what it is not.
    pub fn source(self) -> &'static str {
        match self {
            Self::UiFrames => {
                "frames the window GPUI really painted (one per Window::draw; the diagnostic's own redraw is not counted)"
            }
            Self::UiIntervalMs => {
                "gap between two painted frames of the same run; while playing a long gap is a stall and is kept, at rest a gap longer than 1 s records none"
            }
            Self::MonitorFrames => {
                "new pictures the host accepted (frame.done of the request in flight); not compositor presentation"
            }
            Self::MonitorIntervalMs => {
                "gap between two accepted pictures; while playing, a silence is an interval (stall), at rest it is none"
            }
            Self::RenderMs => {
                "frame.done timings.render_ms of an accepted picture: CPU time until the draw was submitted, excluding GPU rasterization"
            }
            Self::LatencyMs => {
                "host admitted the request until the picture with the same request id was accepted, one process clock; not GPU completion, not seek-to-photon"
            }
            Self::LostFrames => {
                "playback clock frames no accepted picture showed, as the delta since the previous observation of the same playback run"
            }
            Self::RssBytes => {
                "resident set of the editor process; WebKit's WebContent/GPU processes are not included; not GPU memory"
            }
            Self::TextureBytes => {
                "estimate of the retained RGBA image textures the runtime reports (cache.texture_bytes); not a GPU reading"
            }
        }
    }

    /// Series of the same group may share a scale on screen; others never do.
    pub fn group(self) -> &'static str {
        match self {
            Self::UiFrames | Self::MonitorFrames => "rate",
            Self::UiIntervalMs | Self::MonitorIntervalMs | Self::RenderMs | Self::LatencyMs => {
                "time_ms"
            }
            Self::LostFrames => "loss",
            Self::RssBytes | Self::TextureBytes => "memory",
        }
    }

    /// Row title in the panel (pt-BR). The unit is spelled out: a scale is never shared.
    pub fn label(self) -> &'static str {
        match self {
            Self::UiFrames => "Interface: quadros pintados (fps)",
            Self::UiIntervalMs => "Interface: intervalo entre quadros",
            Self::MonitorFrames => "Monitor: imagens aceitas (fps)",
            Self::MonitorIntervalMs => "Monitor: intervalo entre imagens",
            Self::RenderMs => "render_ms (CPU até a submissão)",
            Self::LatencyMs => "Latência pedido → imagem aceita",
            Self::LostFrames => "Perdas na reprodução (quadros)",
            Self::RssBytes => "RSS do editor (sem WebKit)",
            Self::TextureBytes => "Texturas do runtime (estimativa)",
        }
    }

    /// A value of this metric as people read it. Frame counters are per-bucket counts here; the
    /// panel turns them into a rate with [`rate_fps`].
    pub fn format(self, value: f64) -> String {
        match self.unit() {
            "ms" if value < 100.0 => format!("{value:.1} ms").replace('.', ","),
            "ms" => format!("{value:.0} ms"),
            "bytes" => crate::perf::format_mib(value.max(0.0) as u64),
            _ => format!("{value:.0}"),
        }
    }

    /// Intervals and waits: the events that explain a long one happened *before* its sample.
    pub(crate) fn is_wait(self) -> bool {
        matches!(
            self,
            Self::UiIntervalMs | Self::MonitorIntervalMs | Self::LatencyMs
        )
    }
}

/// Count, minimum, sum and maximum of the samples of one metric in one bucket. An empty one
/// (`count == 0`) means "nothing observed", not zero.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Agg {
    pub count: u32,
    pub sum: f64,
    pub min: f64,
    pub max: f64,
    /// When the maximum was sampled (microseconds on the history's clock).
    pub max_at_us: u64,
}

impl Agg {
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub(crate) fn add(&mut self, value: f64, at: Duration) {
        if self.count == 0 || value < self.min {
            self.min = value;
        }
        if self.count == 0 || value > self.max {
            self.max = value;
            self.max_at_us = micros(at);
        }
        self.count = self.count.saturating_add(1);
        self.sum += value;
    }

    /// Weighted by count: the mean of the merged samples, not the mean of the means.
    pub(crate) fn merge(&mut self, other: &Agg) {
        if other.count == 0 {
            return;
        }
        if self.count == 0 || other.min < self.min {
            self.min = other.min;
        }
        if self.count == 0 || other.max > self.max {
            self.max = other.max;
            self.max_at_us = other.max_at_us;
        }
        self.count = self.count.saturating_add(other.count);
        self.sum += other.sum;
    }

    /// `None` for an empty bucket.
    pub fn mean(&self) -> Option<f64> {
        (self.count > 0).then(|| self.sum / f64::from(self.count))
    }
}

/// Frames per second a bucket's frame count stands for (`count / 0.25 s`).
pub fn rate_fps(count: u32) -> f64 {
    f64::from(count) / BUCKET.as_secs_f64()
}

/// One line for an inspected bucket: what was observed, or that nothing was.
pub fn describe(metric: Metric, agg: &Agg) -> String {
    let Some(mean) = agg.mean() else {
        return "sem amostra".to_string();
    };
    match metric {
        Metric::UiFrames | Metric::MonitorFrames => {
            format!(
                "{} quadros · {} fps",
                agg.count,
                format!("{:.0}", rate_fps(agg.count))
            )
        }
        Metric::LostFrames => format!("{} perdidos (em {} leituras)", agg.sum as u64, agg.count),
        _ => format!(
            "{}× · mín {} · méd {} · máx {}",
            agg.count,
            metric.format(agg.min),
            metric.format(mean),
            metric.format(agg.max)
        ),
    }
}

pub(crate) fn micros(at: Duration) -> u64 {
    u64::try_from(at.as_micros()).unwrap_or(u64::MAX)
}

pub(crate) fn ms(at: Duration) -> f64 {
    at.as_secs_f64() * 1000.0
}

/// Index of the bucket `at` falls in.
pub(crate) fn bucket_of(at: Duration) -> u64 {
    u64::try_from(at.as_nanos() / BUCKET.as_nanos()).unwrap_or(u64::MAX)
}

// ---------------------------------------------------------------------------- ring

#[derive(Debug, Clone)]
struct Slot {
    index: Option<u64>,
    aggs: [Agg; METRICS],
}

impl Default for Slot {
    fn default() -> Self {
        Self {
            index: None,
            aggs: [Agg::default(); METRICS],
        }
    }
}

/// 240 slots indexed by `bucket % 240`; a slot belongs to the bucket it names.
#[derive(Debug, Clone)]
struct Ring {
    slots: Vec<Slot>,
    newest: u64,
}

impl Ring {
    fn new() -> Self {
        Self {
            slots: vec![Slot::default(); BUCKETS],
            newest: 0,
        }
    }

    /// `false` when the sample is older than the window ends up being (dropped, counted).
    fn record(&mut self, metric: Metric, at: Duration, value: f64) -> bool {
        if !value.is_finite() {
            return false;
        }
        let index = bucket_of(at);
        if index.saturating_add(BUCKETS as u64) <= self.newest {
            return false;
        }
        self.newest = self.newest.max(index);
        let slot = &mut self.slots[(index % BUCKETS as u64) as usize];
        if slot.index != Some(index) {
            *slot = Slot {
                index: Some(index),
                aggs: [Agg::default(); METRICS],
            };
        }
        slot.aggs[metric.index()].add(value, at);
        true
    }

    /// The aggregates of bucket `index`, empty when it holds nothing (or something else).
    fn at(&self, index: u64) -> [Agg; METRICS] {
        let slot = &self.slots[(index % BUCKETS as u64) as usize];
        if slot.index == Some(index) {
            slot.aggs
        } else {
            [Agg::default(); METRICS]
        }
    }
}

// ---------------------------------------------------------------------------- events

/// Why the history opened a new segment: what was replaced under the measurements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    /// The recording itself began (segment 0).
    Recording,
    /// Cleared on purpose; anchors and deltas restart.
    Cleared,
    ProjectReplaced,
    SceneChanged,
    ViewportReloaded,
}

impl SessionKind {
    pub fn id(self) -> &'static str {
        match self {
            Self::Recording => "recording",
            Self::Cleared => "cleared",
            Self::ProjectReplaced => "project_replaced",
            Self::SceneChanged => "scene_changed",
            Self::ViewportReloaded => "viewport_reloaded",
        }
    }
}

/// What happened, when it was really observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    RecordingStarted,
    RecordingStopped,
    RecordingCleared,
    PlaybackStarted,
    PlaybackStopped,
    Seek,
    ProjectReplaced,
    SceneChanged,
    ViewportReloaded,
    ResourceLoadStarted,
    ResourceLoadFinished,
    ExportStarted,
    ExportFinished,
    ExportFailed,
    ExportCancelled,
    ContextLost,
    ContextRestored,
    RequestFailed,
    /// The runtime, or its page, became unavailable.
    RuntimeUnavailable,
    /// A frame size/quality change.
    FrameFormatChanged,
}

impl EventKind {
    pub fn id(self) -> &'static str {
        match self {
            Self::RecordingStarted => "recording_started",
            Self::RecordingStopped => "recording_stopped",
            Self::RecordingCleared => "recording_cleared",
            Self::PlaybackStarted => "playback_started",
            Self::PlaybackStopped => "playback_stopped",
            Self::Seek => "seek",
            Self::ProjectReplaced => "project_replaced",
            Self::SceneChanged => "scene_changed",
            Self::ViewportReloaded => "viewport_reloaded",
            Self::ResourceLoadStarted => "resource_load_started",
            Self::ResourceLoadFinished => "resource_load_finished",
            Self::ExportStarted => "export_started",
            Self::ExportFinished => "export_finished",
            Self::ExportFailed => "export_failed",
            Self::ExportCancelled => "export_cancelled",
            Self::ContextLost => "context_lost",
            Self::ContextRestored => "context_restored",
            Self::RequestFailed => "request_failed",
            Self::RuntimeUnavailable => "runtime_unavailable",
            Self::FrameFormatChanged => "frame_format_changed",
        }
    }

    /// Short label for a marker or a list row.
    pub fn label(self) -> &'static str {
        match self {
            Self::RecordingStarted => "gravação iniciada",
            Self::RecordingStopped => "gravação parada",
            Self::RecordingCleared => "histórico limpo",
            Self::PlaybackStarted => "play",
            Self::PlaybackStopped => "pausa/stop",
            Self::Seek => "seek",
            Self::ProjectReplaced => "projeto trocado",
            Self::SceneChanged => "cena trocada",
            Self::ViewportReloaded => "viewport recarregado",
            Self::ResourceLoadStarted => "carga de recurso",
            Self::ResourceLoadFinished => "recurso carregado",
            Self::ExportStarted => "export iniciado",
            Self::ExportFinished => "export concluído",
            Self::ExportFailed => "export falhou",
            Self::ExportCancelled => "export cancelado",
            Self::ContextLost => "contexto perdido",
            Self::ContextRestored => "contexto restaurado",
            Self::RequestFailed => "pedido falhou",
            Self::RuntimeUnavailable => "runtime indisponível",
            Self::FrameFormatChanged => "formato do quadro",
        }
    }
}

/// One thing that happened, on the history's clock.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub at: Duration,
    pub kind: EventKind,
    /// Segment (opaque session id) the event belongs to.
    pub segment: u32,
    /// Sequence frame it concerns, when it has one.
    pub frame: Option<i64>,
    /// Host request id it correlates with, when it has one.
    pub request: Option<u64>,
    /// An error code or a count, never free text: see [`code`].
    pub detail: Option<String>,
    /// How many consecutive occurrences were folded into this one (seeks while scrubbing).
    pub repeat: u32,
}

impl Event {
    pub fn to_json(&self) -> Value {
        json!({
            "at_ms": round3(ms(self.at)), "kind": self.kind.id(), "segment": self.segment,
            "frame": self.frame, "request": self.request, "detail": self.detail,
            "repeat": self.repeat,
        })
    }
}

/// A stretch of the recording measured against one session: nothing crosses its edges.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    pub id: u32,
    pub start: Duration,
    pub kind: SessionKind,
}

/// An error code or a name worth keeping: ASCII letters, digits, `_`, `-`, `.`, at most 48
/// characters. Anything else (a path, a title, a message) is dropped, so an event can never
/// carry project text or a private path.
pub fn code(text: &str) -> Option<String> {
    let clean = !text.is_empty()
        && text.len() <= 48
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    clean.then(|| text.to_string())
}

pub(crate) fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

// ---------------------------------------------------------------------------- context

/// What the pictures were being made of, as of the newest accepted one. No project text, no
/// paths: the session is an opaque number.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Context {
    pub width: u32,
    pub height: u32,
    /// `preview` / `original`, only when it is a plain code.
    pub quality: Option<String>,
    /// Layers drawn from a proxy and with another camera's splat order, as the runtime says.
    pub proxy_layers: u64,
    pub stale_sort_layers: u64,
    pub reduced_scene_layers: Option<u64>,
    /// Playing (a clock is running) or not.
    pub playing: bool,
    /// Whether the native monitor view is on screen (a dialog hides it).
    pub monitor_on_screen: Option<bool>,
    /// Target of the sequence (24 for a 24 fps video), when known. Not the UI's target.
    pub sequence_fps: Option<f64>,
    /// Target of the interface when known (never assumed to be 60).
    pub ui_target_fps: Option<f64>,
}

impl Context {
    pub fn to_json(&self, segment: u32) -> Value {
        json!({
            "session": segment,
            "width": self.width, "height": self.height, "quality": self.quality,
            "proxy_layers": self.proxy_layers, "stale_sort_layers": self.stale_sort_layers,
            "reduced_scene_layers": self.reduced_scene_layers,
            "playing": self.playing, "monitor_on_screen": self.monitor_on_screen,
            "sequence_target_fps": self.sequence_fps, "ui_target_fps": self.ui_target_fps,
        })
    }
}

/// What one accepted picture tells the history.
#[derive(Debug, Clone, Default)]
pub struct Image {
    /// Request id the host admitted (`frame.id`); `None` for a picture that has none.
    pub request: Option<u64>,
    /// Sequence frame shown, when it has one.
    pub frame: Option<i64>,
    pub render_ms: Option<f64>,
    pub texture_bytes: Option<u64>,
}

// ---------------------------------------------------------------------------- recorder

/// Whether and since when the history records.
#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    /// Never started (or cleared while stopped): nothing to read.
    Off,
    Recording {
        started: Duration,
    },
    /// Stopped: what was kept can still be read, anchored at `stopped`, until it is cleared or a
    /// new recording starts.
    Stopped {
        started: Duration,
        stopped: Duration,
    },
}

/// Counters of what the recorder refused or dropped, so a hole in the data has a reason.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    /// Answers to a request that was superseded or belonged to an older session.
    pub stale_responses: u64,
    /// Pictures whose request id was not admitted (or was forgotten): no latency was invented.
    pub unmatched_images: u64,
    /// Admitted requests forgotten because more than [`MAX_PENDING`] waited.
    pub forgotten_requests: u64,
    /// Samples older than the window the ring had already moved to.
    pub late_samples: u64,
    /// Memory readings the platform could not give.
    pub memory_unavailable: u64,
}

/// The recorder: state machine, ring, events, segments and the anchors the deltas rely on.
#[derive(Debug, Clone)]
pub struct Recorder {
    state: State,
    ring: Option<Box<Ring>>,
    frozen: Option<Snapshot>,
    selected: Option<Duration>,
    // anchors
    ui_last: Option<Duration>,
    monitor_last: Option<Duration>,
    /// Frames are expected since this moment (a playback clock is running).
    motion_since: Option<Duration>,
    loss: Option<(u64, u64)>,
    pending: VecDeque<(u64, Duration)>,
    rss_tried: Option<Duration>,
    texture_tried: Option<Duration>,
    events: VecDeque<Event>,
    dropped_events: u64,
    segments: VecDeque<Segment>,
    segment: u32,
    counters: Counters,
    context: Option<Context>,
    /// The graphics context is down: a loss was observed and no restore yet. Kept as a flag (not
    /// read back from the event ring) so a loss longer than the window, whose event aged out,
    /// is still one loss when the page repeats its notice.
    context_down: bool,
    /// Frame format of the last picture, to emit one event per change.
    format: Option<(u32, u32, Option<String>)>,
    /// Export jobs as of the last look at the registry (`None`: no baseline yet, so what already
    /// ran before the recording is not announced as news).
    exports: Option<HashMap<String, JobState>>,
    exports_tried: Option<Duration>,
}

impl Default for Recorder {
    fn default() -> Self {
        Self {
            state: State::Off,
            ring: None,
            frozen: None,
            selected: None,
            ui_last: None,
            monitor_last: None,
            motion_since: None,
            loss: None,
            pending: VecDeque::new(),
            rss_tried: None,
            texture_tried: None,
            events: VecDeque::new(),
            dropped_events: 0,
            segments: VecDeque::new(),
            segment: 0,
            counters: Counters::default(),
            context: None,
            context_down: false,
            format: None,
            exports: None,
            exports_tried: None,
        }
    }
}

impl Recorder {
    pub fn recording(&self) -> bool {
        matches!(self.state, State::Recording { .. })
    }

    /// Whether there is anything to read (recording, or stopped with its data kept).
    pub fn has_data(&self) -> bool {
        !matches!(self.state, State::Off)
    }

    pub fn frozen(&self) -> Option<&Snapshot> {
        self.frozen.as_ref()
    }

    pub fn selected(&self) -> Option<Duration> {
        self.selected
    }

    pub fn counters(&self) -> Counters {
        self.counters
    }

    pub fn events_held(&self) -> usize {
        self.events.len()
    }

    pub fn dropped_events(&self) -> u64 {
        self.dropped_events
    }

    pub fn pending_requests(&self) -> usize {
        self.pending.len()
    }

    pub fn segment(&self) -> u32 {
        self.segment
    }

    /// Starts a *new* recording at `now`: whatever an earlier one kept is discarded. `playing`
    /// is whether a playback clock is running right now. `false` when already recording.
    ///
    /// A recording that begins during playback did **not** see the play happen, so no
    /// `playback_started` event is made up: the context it begins in is stated on the start
    /// event itself (`detail: "playing"`) and frames are expected from `now` (so a stall is
    /// measured from here, never from a play nobody observed). A later pause is a real event.
    pub fn start(&mut self, now: Duration, playing: bool) -> bool {
        if self.recording() {
            return false;
        }
        *self = Self {
            state: State::Recording { started: now },
            ring: Some(Box::new(Ring::new())),
            motion_since: playing.then_some(now),
            ..Self::default()
        };
        self.open_segment(now, SessionKind::Recording);
        self.push(
            now,
            EventKind::RecordingStarted,
            None,
            None,
            playing.then(|| PLAYING_CONTEXT.to_string()),
        );
        true
    }

    /// Stops collecting: no new sample is accepted, the data is kept and readable (anchored at
    /// `now`), a frozen snapshot stays. `false` when it was not recording.
    pub fn stop(&mut self, now: Duration) -> bool {
        let State::Recording { started } = self.state else {
            return false;
        };
        self.push(now, EventKind::RecordingStopped, None, None, None);
        self.state = State::Stopped {
            started,
            stopped: now,
        };
        self.ui_last = None;
        self.monitor_last = None;
        self.motion_since = None;
        self.loss = None;
        self.pending.clear();
        true
    }

    /// Forgets everything, **explicitly** (the user asked): the ring, the events, the segments,
    /// the anchors and the deltas, the selection and the frozen snapshot, so nothing a reader
    /// holds survives it (the document of a cleared history is gone, not "kept frozen"). Whether
    /// a freeze was released is reported by [`Self::apply`]. A running recording continues from
    /// `now` with fresh anchors (no delta against a value from before); a stopped one becomes
    /// empty. The project is untouched. As in [`Self::start`], a clear during playback states
    /// the context on the event and does not invent a `playback_started`.
    pub fn clear(&mut self, now: Duration, playing: bool) {
        match self.state {
            State::Recording { .. } => {
                *self = Self {
                    state: State::Recording { started: now },
                    ring: Some(Box::new(Ring::new())),
                    motion_since: playing.then_some(now),
                    ..Self::default()
                };
                self.open_segment(now, SessionKind::Cleared);
                self.push(
                    now,
                    EventKind::RecordingCleared,
                    None,
                    None,
                    playing.then(|| PLAYING_CONTEXT.to_string()),
                );
            }
            State::Stopped { .. } | State::Off => *self = Self::default(),
        }
    }

    // ------------------------------------------------------------------ feeds

    fn live(&self) -> bool {
        self.recording() && self.ring.is_some()
    }

    fn sample(&mut self, metric: Metric, at: Duration, value: f64) {
        let Some(ring) = &mut self.ring else {
            return;
        };
        if !ring.record(metric, at, value) {
            self.counters.late_samples += 1;
        }
    }

    /// Interval to the previous frame of `source`, honouring rests and expected motion.
    fn interval(&self, last: Option<Duration>, at: Duration) -> Option<Duration> {
        let since = match (last, self.motion_since) {
            (Some(last), Some(motion)) => Some(last.max(motion)),
            (None, Some(motion)) => Some(motion),
            (last, None) => last,
        }?;
        if at < since {
            return None;
        }
        let gap = at - since;
        // Expected motion: a long silence is a stall and stays. Otherwise a pause is a rest:
        // two runs, not one huge interval.
        (self.motion_since.is_some() || gap <= REST_AFTER).then_some(gap)
    }

    /// The window painted a frame (one `Window::draw`). The adapter never calls this for the
    /// diagnostic's own redraw.
    ///
    /// The interval to the previous paint follows the same rest/stall rule as the monitor's:
    /// while a playback clock runs the window is expected to keep painting, so a long gap is a
    /// **real** stall and is recorded whatever its length (the maximum and the events that
    /// happened during it explain it); at rest, a gap longer than [`REST_AFTER`] is a pause in
    /// the activity and records nothing. Nothing is measured against an anchor that does not
    /// exist: the first paint of a recording, of a cleared history or of a new segment only
    /// *becomes* the anchor. A playback that starts after a long idle measures from its start.
    pub fn ui_painted(&mut self, at: Duration) {
        if !self.live() {
            return;
        }
        self.sample(Metric::UiFrames, at, 1.0);
        if self.ui_last.is_some()
            && let Some(gap) = self.interval(self.ui_last, at)
        {
            self.sample(Metric::UiIntervalMs, at, ms(gap));
        }
        self.ui_last = Some(at);
    }

    /// The host accepted a picture (no further detail known).
    pub fn monitor_presented(&mut self, at: Duration) {
        if !self.live() {
            return;
        }
        self.sample(Metric::MonitorFrames, at, 1.0);
        if let Some(gap) = self.interval(self.monitor_last, at) {
            self.sample(Metric::MonitorIntervalMs, at, ms(gap));
        }
        self.monitor_last = Some(at);
    }

    /// The host accepted the picture of a request it had admitted. CPU time, latency, the
    /// texture gauge and the frame format are recorded once, here: a picture is one sample, not
    /// one per later refresh.
    pub fn image(&mut self, at: Duration, image: &Image) {
        if !self.live() {
            return;
        }
        self.monitor_presented(at);
        if let Some(render) = image.render_ms {
            self.sample(Metric::RenderMs, at, render);
        }
        match image.request {
            Some(id) => self.match_request(at, id),
            None => self.counters.unmatched_images += 1,
        }
        if let Some(bytes) = image.texture_bytes
            && self.gauge_due(self.texture_tried, at)
        {
            self.texture_tried = Some(at);
            self.sample(Metric::TextureBytes, at, bytes as f64);
        }
    }

    fn match_request(&mut self, at: Duration, id: u64) {
        let Some(position) = self.pending.iter().position(|(pending, _)| *pending == id) else {
            // Never admitted here, forgotten, or from an older session: no latency is invented.
            self.counters.unmatched_images += 1;
            return;
        };
        let (_, admitted) = self.pending[position];
        // Requests older than the one answered were superseded and will never be answered.
        self.pending.drain(..=position);
        if at >= admitted {
            self.sample(Metric::LatencyMs, at, ms(at - admitted));
        }
    }

    /// The host admitted a request for a picture.
    pub fn request_admitted(&mut self, id: u64, at: Duration) {
        if !self.live() {
            return;
        }
        if self.pending.iter().any(|(pending, _)| *pending == id) {
            return;
        }
        if self.pending.len() >= MAX_PENDING {
            self.pending.pop_front();
            self.counters.forgotten_requests += 1;
        }
        self.pending.push_back((id, at));
    }

    /// An answer was discarded as obsolete (superseded request or older session).
    pub fn response_stale(&mut self, id: u64) {
        if !self.live() {
            return;
        }
        self.counters.stale_responses += 1;
        self.pending.retain(|(pending, _)| *pending != id);
    }

    /// A request failed: it will never be answered, and the failure is an event.
    pub fn request_failed(&mut self, at: Duration, id: Option<u64>, error_code: &str) {
        if !self.live() {
            return;
        }
        if let Some(id) = id {
            self.pending.retain(|(pending, _)| *pending != id);
        }
        if error_code == "context_lost" {
            // The graphics context went away: that is the event, not just a failed request.
            self.context_lost(at, id);
            return;
        }
        self.push(at, EventKind::RequestFailed, None, id, code(error_code));
    }

    /// The graphics context was lost (a frame failed with `context_lost`, or the page said so).
    /// One event per loss: the page's notice and the failed frame it caused are the same loss,
    /// whichever arrives first. `request` is the request that was in flight, when there was one;
    /// a loss while idle has none.
    pub fn context_lost(&mut self, at: Duration, request: Option<u64>) {
        if !self.live() || self.context_down {
            return;
        }
        self.context_down = true;
        self.push(
            at,
            EventKind::ContextLost,
            None,
            request,
            Some("context_lost".to_string()),
        );
    }

    /// The graphics context came back. Nothing is carried over: the picture it lost is not
    /// reported as shown, and the next accepted picture is a new measurement. A restore is
    /// recorded even when the loss predates the recording (it is observed); a second notice
    /// without a new loss in between is not a second restore.
    pub fn context_restored(&mut self, at: Duration) {
        if !self.live() {
            return;
        }
        let was_down = std::mem::replace(&mut self.context_down, false);
        let already_restored = !was_down
            && self
                .events
                .iter()
                .rev()
                .find_map(|event| match event.kind {
                    EventKind::ContextLost => Some(false),
                    EventKind::ContextRestored => Some(true),
                    _ => None,
                })
                .unwrap_or(false);
        if already_restored {
            return;
        }
        self.push(at, EventKind::ContextRestored, None, None, None);
    }

    /// Whether the export jobs should be looked at now (at most once per second).
    pub fn exports_due(&self, at: Duration) -> bool {
        self.live() && self.gauge_due(self.exports_tried, at)
    }

    /// The export jobs as the registry shows them at `at`. The first look after the recording
    /// began is only the baseline (an export that already ran is not news); after that every
    /// transition seen becomes one event, and a job first seen already finished yields only its
    /// end, since its start was not observed. Times are the moment of the look, so an event can
    /// be late by at most the look interval; they are never earlier than the truth.
    pub fn exports_observed(&mut self, at: Duration, jobs: &[ExportJob]) {
        if !self.live() {
            return;
        }
        self.exports_tried = Some(at);
        let baseline = self.exports.is_none();
        let known = self.exports.take().unwrap_or_default();
        let mut next = HashMap::with_capacity(jobs.len().min(MAX_EXPORT_JOBS));
        for job in jobs.iter().take(MAX_EXPORT_JOBS) {
            let before = known.get(&job.id).copied();
            next.insert(job.id.clone(), job.state);
            if baseline || before == Some(job.state) {
                continue;
            }
            let was_final = before.is_some_and(JobState::is_final);
            match job.state {
                JobState::Queued | JobState::Running if before.is_none() => {
                    self.push(at, EventKind::ExportStarted, None, None, None);
                }
                JobState::Queued | JobState::Running => {}
                _ if was_final => {}
                JobState::Succeeded => self.push(at, EventKind::ExportFinished, None, None, None),
                JobState::Failed => {
                    let detail = job.error_code.as_deref().and_then(code);
                    self.push(at, EventKind::ExportFailed, None, None, detail);
                }
                JobState::Cancelled => self.push(at, EventKind::ExportCancelled, None, None, None),
            }
        }
        self.exports = Some(next);
    }

    /// A playback run began (start, resume after a pause or a seek): its counter starts at zero,
    /// so the first reading is compared against zero and not against another run's total.
    pub fn loss_run_started(&mut self, run: u64) {
        if self.live() {
            self.loss = Some((run, 0));
        }
    }

    /// The cumulative lost-frames counter of playback run `run` read `cumulative` at `at`. The
    /// sample is the delta since the previous reading **of the same run**; a new run starts from
    /// zero, so a seek or a restart never adds the old total again.
    pub fn loss_observed(&mut self, at: Duration, run: u64, cumulative: u64) {
        if !self.live() {
            return;
        }
        // Without an anchor of this run (the recording began mid-run, or it was cleared or
        // re-segmented mid-run) the reading only becomes the baseline: a total from before is
        // not a loss that happened now, and no sample (not even a zero) is invented for it.
        let before = match self.loss {
            Some((known, total)) if known == run => total,
            _ => {
                self.loss = Some((run, cumulative));
                return;
            }
        };
        self.loss = Some((run, cumulative));
        self.sample(
            Metric::LostFrames,
            at,
            cumulative.saturating_sub(before) as f64,
        );
    }

    /// A playback clock began running (or resumed after a pause).
    pub fn playback_started(&mut self, at: Duration, frame: i64) {
        if !self.live() || self.motion_since.is_some() {
            return;
        }
        self.motion_since = Some(at);
        self.push(at, EventKind::PlaybackStarted, Some(frame), None, None);
    }

    /// The playback clock stopped or paused: silence is a rest again.
    pub fn playback_stopped(&mut self, at: Duration) {
        if !self.live() || self.motion_since.is_none() {
            return;
        }
        self.motion_since = None;
        self.loss = None;
        self.push(at, EventKind::PlaybackStopped, None, None, None);
    }

    /// The playhead was moved on purpose. Seeks closer than [`SEEK_COALESCE`] are one event.
    pub fn seeked(&mut self, at: Duration, frame: i64) {
        if !self.live() {
            return;
        }
        if let Some(last) = self.events.back_mut()
            && last.kind == EventKind::Seek
            && last.segment == self.segment
            && at >= last.at
            && at - last.at < SEEK_COALESCE
        {
            last.frame = Some(frame);
            last.repeat = last.repeat.saturating_add(1);
            return;
        }
        self.push(at, EventKind::Seek, Some(frame), None, None);
    }

    /// An observed fact (resource load, export, context loss...).
    pub fn event(&mut self, at: Duration, kind: EventKind, detail: Option<&str>) {
        if !self.live() {
            return;
        }
        match kind {
            // The context events have state (one per loss, one per restore): they go through
            // the methods that keep it, never straight into the ring.
            EventKind::ContextLost => self.context_lost(at, None),
            EventKind::ContextRestored => self.context_restored(at),
            _ => self.push(at, kind, None, None, detail.and_then(code)),
        }
    }

    /// Whether a gauge last tried at `last` is due again.
    fn gauge_due(&self, last: Option<Duration>, at: Duration) -> bool {
        last.is_none_or(|last| at < last || at - last >= GAUGE_EVERY)
    }

    /// Whether the process memory should be read now (at most once per second).
    pub fn rss_due(&self, at: Duration) -> bool {
        self.live() && self.gauge_due(self.rss_tried, at)
    }

    /// The reading `None` (the platform could not give it) leaves the bucket absent; the last
    /// value is never carried forward.
    pub fn rss_read(&mut self, at: Duration, bytes: Option<u64>) {
        if !self.live() {
            return;
        }
        self.rss_tried = Some(at);
        match bytes {
            Some(bytes) => self.sample(Metric::RssBytes, at, bytes as f64),
            None => self.counters.memory_unavailable += 1,
        }
    }

    /// The session under the measurements changed. Opens a segment: anchors reset, requests in
    /// flight are forgotten (their answers are stale), nothing is measured across the edge.
    pub fn session_changed(&mut self, at: Duration, kind: SessionKind) {
        if !self.live() {
            return;
        }
        self.open_segment(at, kind);
        // Expected motion goes on from the edge: the silence before it belongs to the old session.
        if self.motion_since.is_some() {
            self.motion_since = Some(at);
        }
        self.ui_last = None;
        self.monitor_last = None;
        self.loss = None;
        self.pending.clear();
        self.format = None;
        if kind == SessionKind::ViewportReloaded {
            // A new page has a context of its own: whatever state the old one was in is gone.
            self.context_down = false;
        }
        let event = match kind {
            SessionKind::ProjectReplaced => EventKind::ProjectReplaced,
            SessionKind::SceneChanged => EventKind::SceneChanged,
            SessionKind::ViewportReloaded => EventKind::ViewportReloaded,
            SessionKind::Recording | SessionKind::Cleared => return,
        };
        self.push(at, event, None, None, None);
    }

    /// Sets what the pictures are made of; a change of size or quality is an event.
    pub fn set_context(&mut self, at: Duration, mut context: Context) {
        if !self.live() {
            return;
        }
        context.quality = context.quality.as_deref().and_then(code);
        let format = (context.width, context.height, context.quality.clone());
        let changed = self.format.as_ref().is_some_and(|known| *known != format);
        if self.format.is_none() || changed {
            self.format = Some(format.clone());
        }
        if changed {
            let detail = format!("{}x{}", format.0, format.1);
            self.push(at, EventKind::FrameFormatChanged, None, None, Some(detail));
        }
        self.context = Some(context);
    }

    pub fn context(&self) -> Option<&Context> {
        self.context.as_ref()
    }

    fn open_segment(&mut self, at: Duration, kind: SessionKind) {
        if !self.segments.is_empty() {
            self.segment = self.segment.saturating_add(1);
        }
        if self.segments.len() >= MAX_SEGMENTS {
            self.segments.pop_front();
        }
        self.segments.push_back(Segment {
            id: self.segment,
            start: at,
            kind,
        });
    }

    fn push(
        &mut self,
        at: Duration,
        kind: EventKind,
        frame: Option<i64>,
        request: Option<u64>,
        detail: Option<String>,
    ) {
        // Events that left the window are not "discarded": they aged out, as the buckets do.
        while self
            .events
            .front()
            .is_some_and(|event| at.saturating_sub(event.at) > WINDOW)
        {
            self.events.pop_front();
        }
        if self.events.len() >= MAX_EVENTS {
            self.events.pop_front();
            self.dropped_events += 1;
        }
        self.events.push_back(Event {
            at,
            kind,
            segment: self.segment,
            frame,
            request,
            detail,
            repeat: 1,
        });
    }

    // ------------------------------------------------------------------ reading

    /// When the window ends for a reader asking at `now`: a stopped recording is read at the
    /// moment it stopped.
    fn horizon(&self, now: Duration) -> Option<Duration> {
        match self.state {
            State::Off => None,
            State::Recording { .. } => Some(now),
            State::Stopped { stopped, .. } => Some(stopped.min(now)),
        }
    }

    /// A coherent, immutable copy of the last 60 seconds as of `now`. `None` when nothing was
    /// ever recorded. Time alone moves the window: no new frame is needed to evict old data.
    pub fn snapshot(&self, now: Duration) -> Option<Snapshot> {
        let ring = self.ring.as_ref()?;
        let horizon = self.horizon(now)?;
        let (started, ended) = match self.state {
            State::Recording { started } => (started, None),
            State::Stopped { started, stopped } => (started, Some(stopped)),
            State::Off => return None,
        };
        Some(Snapshot::capture(snapshot::Capture {
            ring,
            horizon,
            started,
            ended,
            events: &self.events,
            segments: &self.segments,
            dropped_events: self.dropped_events,
            counters: self.counters,
            context: self.context.clone(),
            segment: self.segment,
            expected_motion: self.motion_since.is_some(),
            open_monitor_ms: self
                .motion_since
                .map(|motion| self.monitor_last.map_or(motion, |last| last.max(motion)))
                .filter(|since| horizon >= *since && ended.is_none())
                .map(|since| ms(horizon - since)),
        }))
    }

    /// What a reader sees: the frozen snapshot when there is one, otherwise the moving window.
    pub fn view(&self, now: Duration) -> Option<Snapshot> {
        self.frozen.clone().or_else(|| self.snapshot(now))
    }

    /// Fixes the current window. One snapshot at a time: freezing again keeps the first one and
    /// says so (`Ok(false)`). Recording continues underneath.
    pub fn freeze(&mut self, now: Duration) -> Result<bool, HistoryError> {
        if self.frozen.is_some() {
            return Ok(false);
        }
        let mut snapshot = self.snapshot(now).ok_or(HistoryError::NoRecording)?;
        snapshot.frozen = true;
        self.frozen = Some(snapshot);
        Ok(true)
    }

    /// Back to the moving history. `false` when nothing was frozen.
    pub fn resume(&mut self) -> bool {
        self.frozen.take().is_some()
    }

    /// Selects an instant (milliseconds on the history's clock) to inspect; `None` clears it.
    pub fn select(&mut self, at: Option<Duration>) {
        self.selected = at;
    }

    /// The numbers, extremes and correlated events at the selected instant, in the view.
    pub fn inspection(&self, now: Duration) -> Option<Inspection> {
        self.view(now)?.inspect(self.selected?)
    }

    /// The exportable document of the view (frozen snapshot when frozen).
    pub fn export_document(&self, now: Duration) -> Result<Value, HistoryError> {
        let snapshot = self.view(now).ok_or(HistoryError::NoRecording)?;
        Ok(snapshot.to_document())
    }

    /// Takes the exportable document of the view **now** and pairs it with `path`, without
    /// touching the disk: an adapter writes it later (off its UI thread) with
    /// [`PreparedExport::write`], and what lands in the file is this coherent copy, whatever the
    /// recorder does meanwhile. Same refusals and precedence as [`Self::export`] for what needs
    /// no disk (`perf_history_empty`, then a missing or relative path); the rest is the write's.
    pub fn prepare_export(
        &self,
        now: Duration,
        path: std::path::PathBuf,
    ) -> Result<PreparedExport, HistoryError> {
        PreparedExport::new(self.export_document(now)?, path)
    }

    /// Writes [`Self::export_document`] to `path` without replacing anything, on the calling
    /// thread (the library and tests; a window uses [`Self::prepare_export`] instead). Nothing
    /// but the new file changes.
    pub fn export(
        &self,
        now: Duration,
        path: &std::path::Path,
    ) -> Result<(Value, usize), HistoryError> {
        let document = self.export_document(now)?;
        let bytes = write_export(path, &document)?;
        Ok((document, bytes))
    }

    /// Compact state for `get`/`snapshot`: flags, range, peaks per metric and the newest events.
    pub fn state_json(&self, now: Duration) -> Value {
        let base = json!({
            "recording": self.recording(),
            "has_data": self.has_data(),
            "frozen": self.frozen.is_some(),
            "selected_ms": self.selected.map(|at| round3(ms(at))),
        });
        let mut state = base;
        if let Some(view) = self.view(now) {
            state["view"] = view.compact_json();
        }
        if let Some(inspection) = self.inspection(now) {
            state["inspection"] = inspection.to_json();
        }
        state
    }

    /// Runs a command that needs only the recorder. Recording on/off and visibility belong to the
    /// adapter (they need the playback clock and the window) and are not handled here.
    pub fn apply(
        &mut self,
        command: &HistoryCommand,
        now: Duration,
    ) -> Result<Value, CommandError> {
        let target = TARGET;
        let result = match command {
            HistoryCommand::Freeze => {
                let changed = self.freeze(now)?;
                json!({ "action": "freeze", "changed": changed, "frozen": true })
            }
            HistoryCommand::Resume => {
                let changed = self.resume();
                json!({ "action": "resume", "changed": changed, "frozen": false })
            }
            HistoryCommand::Select(at) => {
                if let Some(at) = at {
                    let view = self.view(now).ok_or(HistoryError::NoRecording)?;
                    if view.index_at(*at).is_none() {
                        return Err(HistoryError::OutsideWindow {
                            at_ms: round3(ms(*at)),
                            start_ms: round3(view.window_start),
                            end_ms: round3(view.window_end),
                        }
                        .into());
                    }
                }
                self.select(*at);
                json!({ "action": "select", "changed": true,
                    "selected_ms": at.map(|at| round3(ms(at))),
                    "inspection": self.inspection(now).map(|i| i.to_json()) })
            }
            HistoryCommand::Clear => {
                let playing = self.motion_since.is_some();
                // An explicit clear forgets the frozen snapshot too: say so, so a caller never
                // finds out by reading a freeze that is no longer there.
                let released_freeze = self.frozen.is_some();
                self.clear(now, playing);
                json!({ "action": "clear", "changed": true, "released_freeze": released_freeze })
            }
            HistoryCommand::Snapshot => {
                let document = self.export_document(now)?;
                json!({ "action": "snapshot", "changed": false, "snapshot": document })
            }
            HistoryCommand::Export(path) => {
                let (document, bytes) = self.export(now, path)?;
                json!({ "action": "export", "changed": false, "bytes": bytes,
                    "frozen": document["recording"]["frozen"],
                    "range": document["window"] })
            }
            HistoryCommand::Record(_)
            | HistoryCommand::Show
            | HistoryCommand::Hide
            | HistoryCommand::Destination(_)
            | HistoryCommand::ExportToDestination => {
                return Err(CommandError::new(
                    "desktop_required",
                    "Recording, visibility and the export destination are applied by the adapter that owns the clock and the window",
                )
                .target(target));
            }
        };
        let mut result = result;
        result["target"] = json!(target);
        Ok(result)
    }
}

/// The part of an export job the history needs: no label, no path, no result.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportJob {
    pub id: String,
    pub state: JobState,
    /// The job's error code when it failed.
    pub error_code: Option<String>,
}

/// Export jobs remembered between two looks at the registry.
const MAX_EXPORT_JOBS: usize = 64;

/// What a gesture, `ui invoke|set` or a test asks of the history.
#[derive(Debug, Clone, PartialEq)]
pub enum HistoryCommand {
    /// Start (`true`) or stop recording.
    Record(bool),
    /// Open or close the diagnostic panel. Never starts or stops recording.
    Show,
    Hide,
    Freeze,
    Resume,
    /// Select an instant (milliseconds on the history's clock) or clear the selection.
    Select(Option<Duration>),
    /// Forget the history. Explicit; the project is not touched.
    Clear,
    /// Return the exportable document without writing it.
    Snapshot,
    /// Write the document to an explicit path.
    Export(std::path::PathBuf),
    /// Fill the panel's destination field (a path typed or picked). Applied by the window.
    Destination(String),
    /// Write the document to the panel's destination field. Applied by the window, which turns
    /// it into [`Self::Export`]; an empty field is `perf_history_export_path_missing`.
    ExportToDestination,
}
