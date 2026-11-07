//! Rules of the debug performance meter (F049): sampling windows, rates, worst intervals,
//! dropped playback frames, what to show for each state and where the floating card may sit.
//!
//! No GPUI, no clock and no thread: every method takes the time it is told about (a
//! `Duration` since an epoch the adapter chooses), so the rules are testable with a fake clock
//! and the window, the CLI and a Rust caller share them. The adapter feeds two sources:
//!
//! - **UI frames**: one call per frame the window really painted (never a timer tick);
//! - **monitor frames**: one call per picture the viewport accepted from the runtime
//!   (`frame.done` of the request in flight; stale and cancelled answers are not pictures).
//!
//! A hidden meter ([`Meter::disable`]) keeps nothing: every `record` call returns at once, the
//! windows are emptied and [`Meter::next_wakeup`] is `None`, so hiding it costs no timer, no
//! repaint and no memory.
//!
//! What the numbers are **not**: `render_ms` is CPU time until the draw was handed to the GPU
//! process, and the monitor rate is the rate of pictures the runtime *reported finished*. Neither
//! measures the GPU's rasterization nor the compositor (see `docs/features/perf-meter.md`).

use std::collections::VecDeque;
use std::time::Duration;

use serde_json::{Value, json};

use crate::backend::FrameResult;

/// Frames of the last second give the rate.
pub const RATE_WINDOW: Duration = Duration::from_millis(1000);
/// No frame for this long and nothing is expected: the source is *at rest*, not at zero fps.
pub const IDLE_AFTER: Duration = Duration::from_millis(1000);
/// The worst interval is the longest one of the last five seconds.
pub const WORST_WINDOW: Duration = Duration::from_millis(5000);
/// The displayed numbers are recomputed at most this often (4 Hz), whatever paints.
pub const REFRESH_EVERY: Duration = Duration::from_millis(250);
/// Hard cap on remembered frames (a source painting faster than 800 Hz is not a frame rate).
const MAX_FRAMES: usize = 4096;

/// The keystroke GPUI binds: ⌘ on macOS, Ctrl elsewhere (GPUI has no portable "secondary").
pub const SHORTCUT: &str = if cfg!(target_os = "macos") {
    "cmd-shift-d"
} else {
    "ctrl-shift-d"
};
/// The same keystroke as people read it on macOS and elsewhere.
pub const SHORTCUT_LABEL_MAC: &str = "⌘⇧D";
pub const SHORTCUT_LABEL_OTHER: &str = "Ctrl+Shift+D";
/// Automation target of the toggle.
pub const TARGET: &str = "toolbar.perf_meter";

/// What a gesture, the shortcut or `ui invoke|set` asks of the meter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PerfMeterCommand {
    Toggle,
    Show,
    Hide,
}

impl PerfMeterCommand {
    /// The visibility this command leaves behind.
    pub fn apply(self, visible: bool) -> bool {
        match self {
            Self::Toggle => !visible,
            Self::Show => true,
            Self::Hide => false,
        }
    }
}

// ---------------------------------------------------------------- frame rate and intervals

/// What a [`FrameClock`] can say right now.
#[derive(Debug, Clone, PartialEq)]
pub enum Reading {
    /// The source cannot produce frames (no viewport, runtime missing): there is no number.
    Unavailable,
    /// Nothing was presented for [`IDLE_AFTER`] and nothing is expected: a still picture.
    Idle,
    /// Frames are arriving but there are not enough yet for an honest rate.
    Measuring,
    /// Frames per second over the last second and the longest gap of the last five seconds.
    Live { fps: f64, worst_interval_ms: f64 },
}

impl Reading {
    pub fn to_json(&self) -> Value {
        match self {
            Self::Unavailable => json!({ "state": "unavailable" }),
            Self::Idle => json!({ "state": "idle" }),
            Self::Measuring => json!({ "state": "measuring" }),
            Self::Live {
                fps,
                worst_interval_ms,
            } => json!({ "state": "live", "fps": round1(*fps),
                "worst_interval_ms": round1(*worst_interval_ms) }),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Stamp {
    at: Duration,
    /// Gap to the previous frame of the same run; `None` for the first frame after a rest.
    gap: Option<Duration>,
}

/// Frame timestamps of one source and the rate/worst-interval rules over them.
#[derive(Debug, Clone, Default)]
pub struct FrameClock {
    frames: VecDeque<Stamp>,
    last: Option<Duration>,
    /// When motion was last declared (playback started): a stall counts from here.
    anchor: Option<Duration>,
    /// Frames are *expected* (playing): a long gap is a stall, never a rest.
    motion: bool,
}

impl FrameClock {
    /// Declares whether frames are expected. While they are, silence is a stall and is
    /// reported as such; otherwise it is a rest.
    pub fn set_motion(&mut self, on: bool, at: Duration) {
        if on && !self.motion {
            self.anchor = Some(self.anchor.map_or(at, |anchor| anchor.max(at)));
        }
        self.motion = on;
    }

    /// One frame presented at `at`.
    pub fn record(&mut self, at: Duration) {
        // While motion is expected the gap runs from the later of the last frame and the moment
        // motion began: the rest before playback is not an interval between two pictures.
        let since = match (self.last, self.anchor) {
            (Some(last), Some(anchor)) if self.motion => Some(last.max(anchor)),
            (last, _) => last,
        };
        let gap = since
            .filter(|last| at >= *last)
            .map(|last| at - last)
            // A pause longer than the rest threshold is two runs, not one huge interval.
            .filter(|gap| self.motion || *gap <= IDLE_AFTER);
        self.frames.push_back(Stamp { at, gap });
        self.last = Some(at);
        while self.frames.len() > MAX_FRAMES
            || self
                .frames
                .front()
                .is_some_and(|stamp| at.saturating_sub(stamp.at) > WORST_WINDOW)
        {
            self.frames.pop_front();
        }
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Frames currently held (those of the last [`WORST_WINDOW`]).
    pub fn samples(&self) -> usize {
        self.frames.len()
    }

    /// How long until a still source turns idle, so an adapter can wake once to say so.
    /// `None` when motion is expected, nothing was recorded or it already is idle.
    pub fn idle_in(&self, now: Duration) -> Option<Duration> {
        if self.motion {
            return None;
        }
        let since = now.saturating_sub(self.last?);
        (since < IDLE_AFTER).then(|| IDLE_AFTER - since + Duration::from_millis(1))
    }

    pub fn reading(&self, now: Duration) -> Reading {
        let alive = match (self.last, self.anchor) {
            (Some(last), Some(anchor)) => Some(last.max(anchor)),
            (last, anchor) => last.or(anchor),
        };
        if !self.motion {
            match self.last {
                None => return Reading::Idle,
                Some(last) if now.saturating_sub(last) > IDLE_AFTER => return Reading::Idle,
                _ => {}
            }
        }
        let from = now.saturating_sub(RATE_WINDOW);
        let inside: Vec<Duration> = self
            .frames
            .iter()
            .map(|stamp| stamp.at)
            .filter(|at| *at >= from && *at <= now)
            .collect();
        let count = inside.len();
        let worst_recorded = self
            .frames
            .iter()
            .filter(|stamp| now.saturating_sub(stamp.at) <= WORST_WINDOW)
            .filter_map(|stamp| stamp.gap)
            .max();
        if count < 2 {
            // Playing with (almost) no frames for a whole window is a stall, and says so.
            let stall = alive.map(|alive| now.saturating_sub(alive));
            return match stall {
                Some(stall) if self.motion && stall >= RATE_WINDOW => Reading::Live {
                    fps: 0.0,
                    worst_interval_ms: ms(stall.max(worst_recorded.unwrap_or_default())),
                },
                _ => Reading::Measuring,
            };
        }
        let (first, last) = (inside[0], inside[count - 1]);
        let span = last - first;
        if span.is_zero() {
            return Reading::Measuring;
        }
        let mean = span / (count as u32 - 1);
        let trailing = now.saturating_sub(last);
        let stalled = trailing > mean * 2;
        let end = if stalled { now } else { last };
        let fps = (count - 1) as f64 / (end - first).as_secs_f64();
        let worst =
            worst_recorded
                .unwrap_or(mean)
                .max(if stalled { trailing } else { Duration::ZERO });
        Reading::Live {
            fps,
            worst_interval_ms: ms(worst),
        }
    }
}

// ---------------------------------------------------------------- dropped playback frames

/// What the dropped-frame count says.
#[derive(Debug, Clone, PartialEq)]
pub enum LossReading {
    /// Only a running playback has a clock to lose frames against.
    NotPlaying,
    Live {
        /// Frame numbers from the start to the newest one shown (both included).
        span_frames: u64,
        /// Distinct frames of that span that were actually shown.
        presented_frames: u64,
        /// `span - presented`: frames the clock passed that no picture ever showed.
        lost: u64,
        lost_pct: f64,
        /// Frames the clock is ahead of the newest picture (in flight, or stalled).
        lag_frames: u64,
    },
}

impl LossReading {
    pub fn to_json(&self) -> Value {
        match self {
            Self::NotPlaying => json!({ "state": "not_playing" }),
            Self::Live {
                span_frames,
                presented_frames,
                lost,
                lost_pct,
                lag_frames,
            } => json!({ "state": "live", "span_frames": span_frames,
                "presented_frames": presented_frames, "lost": lost,
                "lost_pct": round1(*lost_pct), "lag_frames": lag_frames }),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct LossRun {
    first: i64,
    clock: i64,
    last_shown: i64,
    presented: u64,
}

/// Frames the playback clock passed that no picture showed. The picture in flight is not a loss
/// (it is `lag`); a frame is lost only once a *newer* picture was shown without it.
#[derive(Debug, Clone, Default)]
pub struct PlaybackLoss {
    run: Option<LossRun>,
}

impl PlaybackLoss {
    /// Playback starts (or seeks) at `frame`, which the monitor is already showing.
    pub fn start(&mut self, frame: i64) {
        self.run = Some(LossRun {
            first: frame,
            clock: frame,
            last_shown: frame,
            presented: 1,
        });
    }

    pub fn stop(&mut self) {
        self.run = None;
    }

    /// The clock reached `frame` (backwards moves are seeks: they restart with [`start`](Self::start)).
    pub fn clock(&mut self, frame: i64) {
        if let Some(run) = &mut self.run {
            run.clock = run.clock.max(frame);
        }
    }

    /// A picture of sequence frame `frame` was shown (`None`: a source preview, no frame number).
    pub fn shown(&mut self, frame: Option<i64>) {
        let (Some(run), Some(frame)) = (&mut self.run, frame) else {
            return;
        };
        // A picture of a frame the clock has not reached yet is not part of this run: it was
        // requested before playback started (the playhead was elsewhere) and finished late.
        if frame > run.last_shown && frame >= run.first && frame <= run.clock {
            run.last_shown = frame;
            run.presented += 1;
        }
    }

    pub fn reading(&self) -> LossReading {
        let Some(run) = self.run else {
            return LossReading::NotPlaying;
        };
        let span = (run.last_shown - run.first + 1).max(1) as u64;
        let lost = span.saturating_sub(run.presented);
        LossReading::Live {
            span_frames: span,
            presented_frames: run.presented,
            lost,
            lost_pct: lost as f64 / span as f64 * 100.0,
            lag_frames: (run.clock - run.last_shown).max(0) as u64,
        }
    }
}

// ---------------------------------------------------------------- the picture on screen

/// What the last picture was made of, from the runtime's own `frame.done`. Every field the
/// runtime did not send is `None`: unavailable, never zero.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FrameContext {
    pub width: u32,
    pub height: u32,
    /// `preview` or `original`: the quality the frame was asked for.
    pub quality: Option<String>,
    /// Layers drawn from a preview proxy (0 when the runtime does not say).
    pub proxy_layers: u64,
    /// MSAA samples of the scene targets (`source.scene_samples`, from runtimes that report it).
    pub msaa: Option<u64>,
    /// Scene layers drawn with fewer samples than the project asks for.
    pub reduced_scene_layers: Option<u64>,
    /// CPU time until the draw was submitted; excludes GPU rasterization.
    pub render_ms: f64,
    /// Estimated RGBA bytes of the image textures the runtime retains (`cache.texture_bytes`);
    /// an estimate from image sizes, not a GPU memory reading.
    pub texture_bytes: Option<u64>,
    pub texture_entries: Option<u64>,
}

impl FrameContext {
    pub fn of(frame: &FrameResult) -> Self {
        let source = frame.detail.source.as_ref();
        let cache = frame.detail.cache.as_ref();
        let number = |value: Option<&Value>, key: &str| value.and_then(|v| v[key].as_u64());
        Self {
            width: frame.width,
            height: frame.height,
            quality: source
                .and_then(|source| source["quality"].as_str())
                .map(String::from),
            proxy_layers: frame.detail.proxy_layers(),
            msaa: number(source, "scene_samples"),
            reduced_scene_layers: number(source, "reduced_scene_layers"),
            render_ms: frame.render_ms,
            texture_bytes: number(cache, "texture_bytes"),
            texture_entries: number(cache, "texture_entries"),
        }
    }

    /// `preview`, `original` or `preview + proxy` (a proxy picture is never presented as the original).
    pub fn quality_label(&self) -> String {
        match (self.quality.as_deref(), self.proxy_layers > 0) {
            (Some(quality), true) => format!("{quality} + proxy"),
            (Some(quality), false) => quality.to_string(),
            (None, true) => "proxy".to_string(),
            (None, false) => "qualidade indisponível".to_string(),
        }
    }
}

// ---------------------------------------------------------------- the meter

/// Everything the meter shows at one instant.
#[derive(Debug, Clone, PartialEq)]
pub struct Readout {
    pub ui: Reading,
    pub monitor: Reading,
    pub loss: LossReading,
    pub frame: Option<FrameContext>,
    /// Resident set of the editor process (WebKit's own processes are not included).
    pub rss_bytes: Option<u64>,
    pub playing: bool,
}

/// Sampling state of the meter. Records nothing while disabled.
#[derive(Debug, Clone, Default)]
pub struct Meter {
    enabled: bool,
    ui: FrameClock,
    monitor: FrameClock,
    loss: PlaybackLoss,
    /// Playback runs (tracked even while disabled, so enabling mid-playback is right).
    playing: bool,
}

impl Meter {
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Starts collecting. `playing_from` is the playback clock's frame when one is running.
    pub fn enable(&mut self, now: Duration, playing_from: Option<i64>) {
        self.clear();
        self.enabled = true;
        self.playing = playing_from.is_some();
        if let Some(frame) = playing_from {
            self.start_run(now, frame);
        }
    }

    /// Stops collecting and forgets every sample.
    pub fn disable(&mut self) {
        self.clear();
        self.enabled = false;
    }

    fn clear(&mut self) {
        self.ui.clear();
        self.monitor.clear();
        self.loss.stop();
    }

    fn start_run(&mut self, now: Duration, frame: i64) {
        self.monitor.set_motion(true, now);
        self.loss.start(frame);
    }

    /// Frames held per source, `(window, monitor)`: nothing while disabled.
    pub fn samples(&self) -> (usize, usize) {
        (self.ui.samples(), self.monitor.samples())
    }

    /// The window painted a frame.
    pub fn ui_painted(&mut self, at: Duration) {
        if self.enabled {
            self.ui.record(at);
        }
    }

    /// The viewport accepted a picture; `frame` is its sequence frame when it has one.
    pub fn monitor_presented(&mut self, at: Duration, frame: Option<i64>) {
        if self.enabled {
            self.monitor.record(at);
            self.loss.shown(frame);
        }
    }

    pub fn playback_started(&mut self, now: Duration, frame: i64) {
        self.playing = true;
        if self.enabled {
            self.start_run(now, frame);
        }
    }

    pub fn playback_clock(&mut self, frame: i64) {
        if self.enabled {
            self.loss.clock(frame);
        }
    }

    /// A seek during playback restarts the comparison from the new frame.
    pub fn playback_seeked(&mut self, frame: i64) {
        if self.enabled && self.playing {
            self.loss.start(frame);
        }
    }

    pub fn playback_stopped(&mut self, now: Duration) {
        self.playing = false;
        if self.enabled {
            self.monitor.set_motion(false, now);
            self.loss.stop();
        }
    }

    /// When the adapter should wake once to show that a source came to rest. `None` while
    /// disabled, playing, or when both are already at rest.
    pub fn next_wakeup(&self, now: Duration) -> Option<Duration> {
        if !self.enabled {
            return None;
        }
        [self.ui.idle_in(now), self.monitor.idle_in(now)]
            .into_iter()
            .flatten()
            .min()
    }

    pub fn readout(
        &self,
        now: Duration,
        monitor_available: bool,
        frame: Option<FrameContext>,
        rss_bytes: Option<u64>,
    ) -> Readout {
        Readout {
            ui: self.ui.reading(now),
            monitor: if monitor_available {
                self.monitor.reading(now)
            } else {
                Reading::Unavailable
            },
            loss: self.loss.reading(),
            frame,
            rss_bytes,
            playing: self.playing,
        }
    }
}

// ---------------------------------------------------------------- text

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

/// Mebibytes with one decimal below 100 and none above, `394 MiB`, `12,3 MiB`.
pub fn format_mib(bytes: u64) -> String {
    let mib = bytes as f64 / (1024.0 * 1024.0);
    if mib < 100.0 {
        format!("{mib:.1} MiB").replace('.', ",")
    } else {
        format!("{mib:.0} MiB")
    }
}

fn format_rate(fps: f64) -> String {
    if fps < 10.0 {
        format!("{fps:.1}").replace('.', ",")
    } else {
        format!("{fps:.0}")
    }
}

impl Reading {
    /// The value cell of a rate row: monospace-friendly, never `0 fps` for a still source.
    pub fn text(&self) -> String {
        match self {
            Self::Unavailable => "indisponível".to_string(),
            Self::Idle => "parado".to_string(),
            Self::Measuring => "medindo…".to_string(),
            Self::Live {
                fps,
                worst_interval_ms,
            } => format!(
                "{:>4} fps · pior {:>4.0} ms",
                format_rate(*fps),
                worst_interval_ms
            ),
        }
    }

    fn short(&self) -> String {
        match self {
            Self::Live { fps, .. } => format_rate(*fps),
            other => other.text(),
        }
    }
}

impl LossReading {
    pub fn text(&self) -> String {
        match self {
            Self::NotPlaying => "só em reprodução".to_string(),
            Self::Live {
                span_frames,
                lost,
                lost_pct,
                lag_frames,
                ..
            } => {
                let pct = format!("{lost_pct:.1}").replace('.', ",");
                format!("{lost} de {span_frames} ({pct}%) · atraso {lag_frames}")
            }
        }
    }

    fn short(&self) -> String {
        match self {
            Self::NotPlaying => "n/d".to_string(),
            Self::Live { lost, .. } => lost.to_string(),
        }
    }
}

impl Readout {
    /// The rows of the floating card: `(label, value)`. Labels are fixed; a value that is not
    /// known says so instead of showing zero.
    pub fn rows(&self) -> Vec<(&'static str, String)> {
        let unknown = || "indisponível".to_string();
        let frame = self.frame.as_ref();
        vec![
            ("interface", self.ui.text()),
            ("monitor", self.monitor.text()),
            ("perdidos", self.loss.text()),
            (
                // The name is the honest part: this is CPU time until submission, not smoothness.
                "render_ms",
                frame.map_or_else(unknown, |frame| {
                    format!("{:.0} ms · CPU, sem a GPU", frame.render_ms)
                }),
            ),
            (
                "quadro",
                frame.map_or_else(unknown, |frame| {
                    let mut text = format!(
                        "{}×{} · {}",
                        frame.width,
                        frame.height,
                        frame.quality_label()
                    );
                    if let Some(msaa) = frame.msaa {
                        text.push_str(&format!(" · MSAA {msaa}"));
                    }
                    text
                }),
            ),
            (
                "RSS",
                self.rss_bytes.map_or_else(
                    || "indisponível".to_string(),
                    |bytes| format!("{} · editor, sem WebKit", format_mib(bytes)),
                ),
            ),
            (
                "texturas",
                frame
                    .and_then(|frame| {
                        frame.texture_bytes.map(|bytes| {
                            let entries = frame
                                .texture_entries
                                .map_or(String::new(), |count| format!(" ({count})"));
                            format!("{}{entries} · estimativa", format_mib(bytes))
                        })
                    })
                    .unwrap_or_else(unknown),
            ),
        ]
    }

    /// One line for the status bar, used when no free corner exists.
    pub fn strip(&self) -> String {
        let frame = self.frame.as_ref();
        format!(
            "UI {} · monitor {} · perdidos {} · render_ms {} (CPU) · RSS {}",
            self.ui.short(),
            self.monitor.short(),
            self.loss.short(),
            frame.map_or("n/d".to_string(), |frame| format!("{:.0}", frame.render_ms)),
            self.rss_bytes.map_or("n/d".to_string(), format_mib),
        )
    }

    pub fn to_json(&self) -> Value {
        let frame = self.frame.as_ref();
        json!({
            "ui": self.ui.to_json(),
            "monitor": self.monitor.to_json(),
            "playing": self.playing,
            "playback_loss": self.loss.to_json(),
            "render_ms": frame.map(|frame| json!({
                "value": round1(frame.render_ms),
                "meaning": "CPU time until the draw was submitted; excludes GPU rasterization",
            })),
            "frame": frame.map(|frame| json!({
                "width": frame.width, "height": frame.height,
                "quality": frame.quality, "proxy_layers": frame.proxy_layers,
                "msaa": frame.msaa, "reduced_scene_layers": frame.reduced_scene_layers,
            })),
            "memory": {
                "rss_bytes": self.rss_bytes,
                "rss_scope": "editor process; WebKit's WebContent and GPU processes are separate",
                "runtime_texture_bytes": frame.and_then(|frame| frame.texture_bytes),
                "runtime_texture_entries": frame.and_then(|frame| frame.texture_entries),
                "runtime_texture_kind": "estimate from image sizes, not a GPU reading",
            },
        })
    }
}

// ---------------------------------------------------------------- where the card goes

/// A rectangle in the coordinates of the area the card floats over.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn intersects(&self, other: &Rect) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }
}

pub const CARD_WIDTH: f32 = 264.0;
pub const CARD_HEIGHT: f32 = 148.0;
pub const CARD_MARGIN: f32 = 8.0;

/// Where the meter sits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Placement {
    /// A card against the right edge, `y` from the top of the area.
    Floating { y: f32 },
    /// No free space: one line in the status bar. It cannot hide behind a native view.
    Strip,
}

/// The native WebView paints above every GPUI element, so a card over it would be invisible.
/// `blocked` are the rectangles such views cover (empty when the monitor is hidden). The card
/// stays against the right edge: top first, then bottom, then just below or above each blocked
/// rectangle; when none of those clears every view, the meter falls back to [`Placement::Strip`].
pub fn place(area: (f32, f32), blocked: &[Rect]) -> Placement {
    let (width, height) = area;
    let lowest = height - CARD_MARGIN - CARD_HEIGHT;
    if width < CARD_WIDTH + 2.0 * CARD_MARGIN || lowest < CARD_MARGIN {
        return Placement::Strip;
    }
    let x = width - CARD_MARGIN - CARD_WIDTH;
    let mut candidates = vec![CARD_MARGIN, lowest];
    let mut between: Vec<f32> = blocked
        .iter()
        .flat_map(|view| {
            [
                view.bottom() + CARD_MARGIN,
                view.y - CARD_MARGIN - CARD_HEIGHT,
            ]
        })
        .filter(|y| (CARD_MARGIN..=lowest).contains(y))
        .collect();
    between.sort_by(f32::total_cmp);
    candidates.extend(between);
    candidates
        .into_iter()
        .find(|y| {
            let card = Rect::new(x, *y, CARD_WIDTH, CARD_HEIGHT);
            blocked.iter().all(|view| !card.intersects(view))
        })
        .map_or(Placement::Strip, |y| Placement::Floating { y })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    /// A source delivering one frame every `step` ms from `from` (included) to `to` (excluded).
    fn feed(clock: &mut FrameClock, from: u64, to: u64, step: u64) {
        let mut t = from;
        while t < to {
            clock.record(at(t));
            t += step;
        }
    }

    fn live(reading: Reading) -> (f64, f64) {
        match reading {
            Reading::Live {
                fps,
                worst_interval_ms,
            } => (fps, worst_interval_ms),
            other => panic!("expected a live reading, got {other:?}"),
        }
    }

    #[test]
    fn a_steady_source_reads_its_rate_and_its_interval() {
        let mut clock = FrameClock::default();
        feed(&mut clock, 0, 2000, 40); // 25 fps
        let (fps, worst) = live(clock.reading(at(1980)));
        assert!((fps - 25.0).abs() < 0.5, "{fps}");
        assert!((worst - 40.0).abs() < 0.5, "{worst}");
    }

    #[test]
    fn the_worst_interval_is_the_longest_gap_of_the_last_five_seconds() {
        let mut clock = FrameClock::default();
        feed(&mut clock, 0, 1000, 20);
        clock.record(at(1230)); // a 250 ms hitch (the last frame above is at 980)
        feed(&mut clock, 1250, 2000, 20);
        let (_, worst) = live(clock.reading(at(1990)));
        assert!((worst - 250.0).abs() < 1.0, "{worst}");
        // Still remembered three seconds later, gone once it is more than five seconds old.
        feed(&mut clock, 2000, 4000, 20);
        let (_, worst) = live(clock.reading(at(3990)));
        assert!((worst - 250.0).abs() < 1.0, "{worst}");
        feed(&mut clock, 4000, 6500, 20);
        let (_, worst) = live(clock.reading(at(6490)));
        assert!(worst < 25.0, "the hitch is outside the window: {worst}");
    }

    #[test]
    fn a_still_source_is_idle_never_zero_fps() {
        let mut clock = FrameClock::default();
        assert_eq!(clock.reading(at(500)), Reading::Idle, "nothing yet");
        feed(&mut clock, 0, 1000, 16);
        assert!(matches!(clock.reading(at(1000)), Reading::Live { .. }));
        assert_eq!(clock.reading(at(2500)), Reading::Idle);
        assert_eq!(Reading::Idle.text(), "parado");
    }

    #[test]
    fn expected_motion_turns_silence_into_a_stall_not_a_rest() {
        let mut clock = FrameClock::default();
        clock.set_motion(true, at(0));
        assert_eq!(clock.reading(at(300)), Reading::Measuring);
        feed(&mut clock, 0, 1000, 40);
        // Playing and then nothing for 3 s: a real zero, with the stall as the worst interval.
        let (fps, worst) = live(clock.reading(at(4000)));
        assert_eq!(fps, 0.0);
        assert!(worst >= 3000.0, "{worst}");
        assert_eq!(
            clock.idle_in(at(4000)),
            None,
            "motion never waits to go idle"
        );
    }

    #[test]
    fn a_gap_after_a_rest_is_not_an_interval() {
        let mut clock = FrameClock::default();
        feed(&mut clock, 0, 200, 20);
        feed(&mut clock, 9000, 9500, 20); // the user came back 8.8 s later
        let (_, worst) = live(clock.reading(at(9480)));
        assert!(worst < 25.0, "the rest is not a stutter: {worst}");
    }

    #[test]
    fn a_stall_in_progress_counts_before_the_next_frame_arrives() {
        let mut clock = FrameClock::default();
        feed(&mut clock, 0, 1000, 20);
        let (_, worst) = live(clock.reading(at(1400)));
        assert!(worst >= 400.0, "the 400 ms in progress: {worst}");
    }

    #[test]
    fn one_frame_is_not_enough_for_a_rate() {
        let mut clock = FrameClock::default();
        clock.record(at(100));
        assert_eq!(clock.reading(at(120)), Reading::Measuring);
        assert_eq!(Reading::Measuring.text(), "medindo…");
    }

    #[test]
    fn the_idle_wakeup_is_one_shot_and_only_for_a_source_that_just_moved() {
        let mut clock = FrameClock::default();
        assert_eq!(
            clock.idle_in(at(0)),
            None,
            "nothing recorded, nothing to wait for"
        );
        clock.record(at(100));
        assert_eq!(clock.idle_in(at(400)), Some(at(701)));
        assert_eq!(clock.idle_in(at(1100)), None, "already idle");
    }

    #[test]
    fn the_rest_before_playback_is_not_the_worst_interval() {
        let mut clock = FrameClock::default();
        clock.record(at(100));
        // A long rest, then play: the first picture is measured from the start of the motion.
        clock.set_motion(true, at(30_000));
        clock.record(at(30_040));
        clock.record(at(30_080));
        let (_, worst) = live(clock.reading(at(30_100)));
        assert_eq!(worst, 40.0);
    }

    #[test]
    fn a_pause_is_a_rest_and_resuming_does_not_record_it_as_a_gap() {
        let mut meter = Meter::default();
        meter.enable(at(0), Some(0));
        meter.monitor_presented(at(10), Some(0));
        meter.playback_clock(1);
        meter.monitor_presented(at(50), Some(1));
        // Pause: nothing is expected, so the silence turns into a rest.
        meter.playback_stopped(at(60));
        let paused = meter.readout(at(5_000), true, None, None);
        assert_eq!(paused.monitor, Reading::Idle);
        assert!(!paused.playing);
        // Resume: the run restarts, and the first picture is measured from the resume.
        meter.playback_started(at(9_000), 1);
        for (step, frame) in [(9_040, 2), (9_080, 3), (9_120, 4)] {
            meter.playback_clock(frame);
            meter.monitor_presented(at(step), Some(frame));
        }
        let resumed = meter.readout(at(9_150), true, None, None);
        match resumed.monitor {
            Reading::Live {
                worst_interval_ms, ..
            } => assert!(worst_interval_ms < 100.0, "{worst_interval_ms}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn dropped_frames_are_the_numbers_a_newer_picture_skipped() {
        let mut loss = PlaybackLoss::default();
        assert_eq!(loss.reading(), LossReading::NotPlaying);
        loss.start(10);
        for frame in [11, 12, 14, 15, 18] {
            loss.clock(frame);
            loss.shown(Some(frame));
        }
        // 10..=18 is 9 numbers; shown 10, 11, 12, 14, 15, 18 = 6; skipped 13, 16, 17.
        loss.clock(19);
        assert_eq!(
            loss.reading(),
            LossReading::Live {
                span_frames: 9,
                presented_frames: 6,
                lost: 3,
                lost_pct: 3.0 / 9.0 * 100.0,
                lag_frames: 1,
            }
        );
    }

    #[test]
    fn a_frame_still_in_flight_is_lag_not_loss() {
        let mut loss = PlaybackLoss::default();
        loss.start(0);
        loss.clock(1);
        loss.shown(Some(1));
        loss.clock(2); // asked for, not shown yet
        match loss.reading() {
            LossReading::Live {
                lost, lag_frames, ..
            } => assert_eq!((lost, lag_frames), (0, 1)),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_late_picture_of_the_old_playhead_does_not_swallow_the_run() {
        // Playback starts at 0 while a picture requested for frame 192 (where the playhead was)
        // finishes afterwards: it is ahead of the clock, so it is not a presented frame of this run.
        let mut loss = PlaybackLoss::default();
        loss.start(0);
        loss.shown(Some(192));
        for frame in 1..=5 {
            loss.clock(frame);
            loss.shown(Some(frame));
        }
        assert_eq!(
            loss.reading(),
            LossReading::Live {
                span_frames: 6,
                presented_frames: 6,
                lost: 0,
                lost_pct: 0.0,
                lag_frames: 0,
            }
        );
    }

    #[test]
    fn repeats_backwards_frames_and_source_previews_do_not_count() {
        let mut loss = PlaybackLoss::default();
        loss.start(5);
        loss.clock(6);
        loss.shown(Some(6));
        loss.shown(Some(6)); // the refine of the same frame
        loss.shown(Some(3)); // before the start
        loss.shown(None); // a source preview has no sequence frame
        match loss.reading() {
            LossReading::Live {
                presented_frames,
                lost,
                ..
            } => assert_eq!((presented_frames, lost), (2, 0)),
            other => panic!("{other:?}"),
        }
        loss.stop();
        assert_eq!(loss.reading(), LossReading::NotPlaying);
    }

    #[test]
    fn a_disabled_meter_records_nothing_and_never_wakes() {
        let mut meter = Meter::default();
        meter.ui_painted(at(10));
        meter.monitor_presented(at(10), Some(1));
        meter.playback_clock(3);
        assert_eq!(meter.next_wakeup(at(20)), None);
        let readout = meter.readout(at(20), true, None, None);
        assert_eq!(readout.ui, Reading::Idle);
        assert_eq!(readout.monitor, Reading::Idle);
        assert_eq!(readout.loss, LossReading::NotPlaying);
    }

    #[test]
    fn enabling_starts_empty_and_disabling_forgets() {
        let mut meter = Meter::default();
        meter.enable(at(0), None);
        feed_meter_ui(&mut meter, 0, 1000, 16);
        assert!(matches!(
            meter.readout(at(1000), true, None, None).ui,
            Reading::Live { .. }
        ));
        meter.disable();
        assert!(!meter.enabled());
        assert_eq!(meter.readout(at(1000), true, None, None).ui, Reading::Idle);
        meter.enable(at(2000), None);
        assert_eq!(
            meter.readout(at(2000), true, None, None).ui,
            Reading::Idle,
            "nothing carried over from the previous session"
        );
    }

    fn feed_meter_ui(meter: &mut Meter, from: u64, to: u64, step: u64) {
        let mut t = from;
        while t < to {
            meter.ui_painted(at(t));
            t += step;
        }
    }

    #[test]
    fn the_meter_wakes_once_for_the_nearest_source_to_rest() {
        let mut meter = Meter::default();
        meter.enable(at(0), None);
        meter.ui_painted(at(100));
        meter.monitor_presented(at(300), None);
        assert_eq!(meter.next_wakeup(at(400)), Some(at(701)));
        meter.playback_started(at(400), 0);
        // Playing: the monitor is expected to move; only the UI source can still rest.
        assert_eq!(meter.next_wakeup(at(400)), Some(at(701)));
        meter.playback_stopped(at(500));
        assert!(meter.next_wakeup(at(500)).is_some());
    }

    #[test]
    fn playback_loss_flows_through_the_meter_only_while_playing() {
        let mut meter = Meter::default();
        meter.enable(at(0), None);
        assert_eq!(
            meter.readout(at(0), true, None, None).loss,
            LossReading::NotPlaying
        );
        meter.playback_started(at(0), 100);
        for (t, frame) in [(40, 101), (80, 102), (160, 104)] {
            meter.playback_clock(frame);
            meter.monitor_presented(at(t), Some(frame));
        }
        match meter.readout(at(200), true, None, None).loss {
            LossReading::Live { lost, .. } => assert_eq!(lost, 1),
            other => panic!("{other:?}"),
        }
        meter.playback_seeked(500);
        assert!(matches!(
            meter.readout(at(210), true, None, None).loss,
            LossReading::Live { lost: 0, .. }
        ));
        meter.playback_stopped(at(220));
        assert_eq!(
            meter.readout(at(230), true, None, None).loss,
            LossReading::NotPlaying
        );
    }

    #[test]
    fn enabling_in_the_middle_of_playback_starts_from_the_clock() {
        let mut meter = Meter::default();
        meter.playback_started(at(0), 0); // meter hidden
        meter.enable(at(5000), Some(120));
        let readout = meter.readout(at(5100), true, None, None);
        assert!(readout.playing);
        assert!(matches!(readout.loss, LossReading::Live { lost: 0, .. }));
    }

    #[test]
    fn an_unavailable_viewport_has_no_monitor_number() {
        let meter = Meter::default();
        let readout = meter.readout(at(0), false, None, None);
        assert_eq!(readout.monitor, Reading::Unavailable);
        assert_eq!(readout.monitor.text(), "indisponível");
    }

    fn done(source: Value, cache: Value) -> FrameResult {
        use crate::backend::FrameDetail;
        FrameResult {
            id: 1,
            revision: 1,
            width: 960,
            height: 540,
            hash: String::new(),
            mean: [0.0; 4],
            probes: Vec::new(),
            png: None,
            render_ms: 16.4,
            total_ms: 17.0,
            detail: FrameDetail {
                source: (!source.is_null()).then_some(source),
                cache: (!cache.is_null()).then_some(cache),
                ..FrameDetail::default()
            },
        }
    }

    #[test]
    fn the_frame_context_reads_what_the_runtime_reports_and_nothing_else() {
        let frame = done(
            json!({ "quality": "preview", "proxy_layers": 1, "original_layers": 0,
                "scene_samples": 0, "reduced_scene_layers": 2 }),
            json!({ "texture_bytes": 8_388_608u64, "texture_entries": 4 }),
        );
        let context = FrameContext::of(&frame);
        assert_eq!((context.width, context.height), (960, 540));
        assert_eq!(context.quality_label(), "preview + proxy");
        assert_eq!(
            context.msaa,
            Some(0),
            "MSAA 0 is a value, not a missing field"
        );
        assert_eq!(context.reduced_scene_layers, Some(2));
        assert_eq!(context.texture_bytes, Some(8_388_608));

        // A runtime without the newer fields: unknown, not zero.
        let older = FrameContext::of(&done(json!({ "quality": "original" }), Value::Null));
        assert_eq!(older.msaa, None);
        assert_eq!(older.texture_bytes, None);
        assert_eq!(older.quality_label(), "original");
    }

    fn readout(frame: Option<FrameContext>, rss: Option<u64>) -> Readout {
        Readout {
            ui: Reading::Live {
                fps: 59.6,
                worst_interval_ms: 18.2,
            },
            monitor: Reading::Idle,
            loss: LossReading::NotPlaying,
            frame,
            rss_bytes: rss,
            playing: false,
        }
    }

    #[test]
    fn rows_say_unavailable_instead_of_zero_and_name_render_ms_for_what_it_is() {
        let none = readout(None, None).rows();
        let row = |rows: &[(&'static str, String)], label: &str| {
            rows.iter().find(|row| row.0 == label).unwrap().1.clone()
        };
        assert_eq!(row(&none, "monitor"), "parado");
        assert_eq!(row(&none, "render_ms"), "indisponível");
        assert_eq!(row(&none, "RSS"), "indisponível");
        assert_eq!(row(&none, "texturas"), "indisponível");
        assert_eq!(row(&none, "perdidos"), "só em reprodução");
        assert!(row(&none, "interface").contains("60 fps"));

        let frame = FrameContext::of(&done(
            json!({ "quality": "original", "scene_samples": 4 }),
            json!({ "texture_bytes": 1_048_576u64, "texture_entries": 2 }),
        ));
        let full = readout(Some(frame), Some(412 * 1024 * 1024)).rows();
        let render = row(&full, "render_ms");
        assert!(render.contains("CPU, sem a GPU"), "{render}");
        assert!(
            !render.contains("fps"),
            "render_ms is never offered as a frame rate"
        );
        assert_eq!(row(&full, "quadro"), "960×540 · original · MSAA 4");
        assert!(row(&full, "RSS").starts_with("412 MiB"));
        assert!(row(&full, "texturas").starts_with("1,0 MiB (2)"));
    }

    #[test]
    fn msaa_is_shown_only_when_the_runtime_reports_it() {
        let frame = FrameContext::of(&done(json!({ "quality": "preview" }), Value::Null));
        let rows = readout(Some(frame), None).rows();
        let quadro = &rows.iter().find(|row| row.0 == "quadro").unwrap().1;
        assert_eq!(quadro, "960×540 · preview");
    }

    #[test]
    fn the_strip_is_one_line_with_the_same_honesty() {
        let strip = readout(None, None).strip();
        assert!(strip.contains("UI 60"));
        assert!(strip.contains("monitor parado"));
        assert!(strip.contains("render_ms n/d"));
        assert!(!strip.contains("render_ms 0"));
    }

    #[test]
    fn json_marks_every_unknown_value_null_and_explains_render_ms() {
        let json = readout(None, None).to_json();
        assert_eq!(json["ui"]["state"], "live");
        assert_eq!(json["monitor"]["state"], "idle");
        assert!(json["render_ms"].is_null());
        assert!(json["memory"]["rss_bytes"].is_null());
        let frame = FrameContext::of(&done(json!({ "quality": "preview" }), Value::Null));
        let json = readout(Some(frame), Some(1)).to_json();
        assert!(
            json["render_ms"]["meaning"]
                .as_str()
                .unwrap()
                .contains("excludes GPU")
        );
        assert!(json["frame"]["msaa"].is_null());
    }

    #[test]
    fn mebibytes_are_formatted_for_a_monospace_cell() {
        assert_eq!(format_mib(0), "0,0 MiB");
        assert_eq!(format_mib(5 * 1024 * 1024 / 2), "2,5 MiB");
        assert_eq!(format_mib(394 * 1024 * 1024), "394 MiB");
    }

    #[test]
    fn commands_toggle_show_and_hide() {
        assert!(PerfMeterCommand::Toggle.apply(false));
        assert!(!PerfMeterCommand::Toggle.apply(true));
        assert!(PerfMeterCommand::Show.apply(true));
        assert!(!PerfMeterCommand::Hide.apply(true));
    }

    // ---- placement

    const AREA: (f32, f32) = (1440.0, 800.0);

    #[test]
    fn with_nothing_native_below_the_card_sits_top_right() {
        assert_eq!(place(AREA, &[]), Placement::Floating { y: CARD_MARGIN });
        // The default layout: the picture is far from the right edge (the inspector is there).
        let picture = Rect::new(300.0, 30.0, 700.0, 400.0);
        assert_eq!(
            place(AREA, &[picture]),
            Placement::Floating { y: CARD_MARGIN }
        );
    }

    #[test]
    fn a_picture_under_the_corner_moves_the_card_to_the_free_bottom_right() {
        // Inspector closed: the picture reaches the right edge, the timeline is below it.
        let picture = Rect::new(240.0, 30.0, 1190.0, 380.0);
        match place(AREA, &[picture]) {
            Placement::Floating { y } => {
                let card = Rect::new(
                    AREA.0 - CARD_MARGIN - CARD_WIDTH,
                    y,
                    CARD_WIDTH,
                    CARD_HEIGHT,
                );
                assert!(!card.intersects(&picture));
                assert!(y > picture.bottom(), "{y}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_maximized_monitor_leaves_no_free_corner_and_the_meter_goes_to_the_status_bar() {
        let picture = Rect::new(8.0, 36.0, 1424.0, 700.0);
        assert_eq!(place((1440.0, 800.0), &[picture]), Placement::Strip);
    }

    #[test]
    fn a_gap_between_views_is_found_and_a_tiny_area_falls_back() {
        // Two views with a band between them just tall enough.
        let top = Rect::new(0.0, 0.0, 1440.0, 200.0);
        let bottom = Rect::new(0.0, 200.0 + CARD_HEIGHT + 2.0 * CARD_MARGIN, 1440.0, 400.0);
        assert_eq!(
            place(AREA, &[top, bottom]),
            Placement::Floating {
                y: 200.0 + CARD_MARGIN
            }
        );
        assert_eq!(place((200.0, 800.0), &[]), Placement::Strip);
        assert_eq!(place((1440.0, 100.0), &[]), Placement::Strip);
    }

    #[test]
    fn touching_edges_do_not_count_as_overlap() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(10.0, 0.0, 10.0, 10.0);
        assert!(!a.intersects(&b));
        assert!(a.intersects(&Rect::new(9.0, 9.0, 5.0, 5.0)));
    }
}
