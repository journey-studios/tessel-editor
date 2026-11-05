//! The device-independent half of playback: what a data callback writes, and what the listener is
//! hearing *now*, both expressed on one sample clock.
//!
//! The clock is the source's own sample index. Each callback says "the first frame of this buffer is
//! source frame `f`, and it reaches the speaker `latency` after `at`"; from that the position that
//! is audible at any later wall time follows without ever asking the device again. The wall clock
//! only extrapolates between two callbacks (a few milliseconds), it never decides where playback is.
//!
//! Nothing here touches a device, so every rule below (seek, pause, resampling, the end, the
//! latency compensation) is tested with synthetic timestamps in this file.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// A callback later than this many buffer periods after the previous one is counted as an
/// underrun suspect: the device asked for audio and the producer was not there in time.
const LATE_FACTOR: f64 = 1.75;

/// Interleaved stereo samples at `sample_rate`: the mix the core's audio plan produced.
#[derive(Clone)]
pub struct Source {
    pub samples: Arc<[f32]>,
    pub sample_rate: u32,
}

impl Source {
    pub fn frames(&self) -> u64 {
        (self.samples.len() / 2) as u64
    }
}

/// Where the listener is, as of the last callback.
#[derive(Clone, Copy, Debug, Default)]
struct Timing {
    /// Source frame of the first output frame of the last callback.
    source_frame: f64,
    /// When that callback ran, since [`Shared::base`].
    at: Duration,
    /// How long after `at` its first frame reaches the speaker.
    latency: Duration,
    /// The position never reads below this: a start or a seek lands here even though the previous
    /// buffers are still being heard for `latency`.
    floor: f64,
    /// Position held while paused or finished.
    held: f64,
    /// `false` once paused or exhausted: the position stops extrapolating.
    running: bool,
    /// The source ran out; the position reaches the end `latency` after `ended_at`.
    ended_at: Option<Duration>,
    seen: bool,
}

/// Counters and the clock, shared between the data callback and everyone else. The callback only
/// ever does atomic operations and a `try_lock`, so it never waits on a reader.
pub struct Shared {
    pub(crate) base: Instant,
    pub(crate) source_rate: u32,
    pub(crate) total_frames: u64,
    pub(crate) start_frame: u64,
    paused: AtomicBool,
    /// Frame to jump to at the next callback; negative: none.
    seek: AtomicI64,
    callbacks: AtomicU64,
    late_callbacks: AtomicU64,
    device_underruns: AtomicU64,
    /// Device frames written while playing (paused silence is not counted).
    frames_out: AtomicU64,
    max_gap_us: AtomicU64,
    buffer_frames: AtomicU64,
    timing: Mutex<Timing>,
}

impl Shared {
    pub(crate) fn new(source: &Source, start_frame: u64) -> Self {
        let total = source.frames();
        let start = start_frame.min(total);
        Self {
            base: Instant::now(),
            source_rate: source.sample_rate,
            total_frames: total,
            start_frame: start,
            paused: AtomicBool::new(false),
            seek: AtomicI64::new(-1),
            callbacks: AtomicU64::new(0),
            late_callbacks: AtomicU64::new(0),
            device_underruns: AtomicU64::new(0),
            frames_out: AtomicU64::new(0),
            max_gap_us: AtomicU64::new(0),
            buffer_frames: AtomicU64::new(0),
            timing: Mutex::new(Timing {
                source_frame: start as f64,
                floor: start as f64,
                held: start as f64,
                ..Timing::default()
            }),
        }
    }

    /// The source's sample rate: the unit of [`Shared::position`].
    pub fn sample_rate(&self) -> u32 {
        self.source_rate
    }

    pub fn total_frames(&self) -> u64 {
        self.total_frames
    }

    pub fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
    }

    pub fn resume(&self) {
        self.paused.store(false, Ordering::SeqCst);
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    /// Jumps to `frame` (clamped to the source) at the next callback, keeping the transport state.
    pub fn seek(&self, frame: u64) {
        self.seek
            .store(frame.min(self.total_frames) as i64, Ordering::SeqCst);
    }

    pub fn device_underrun(&self) {
        self.device_underruns.fetch_add(1, Ordering::Relaxed);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Timing> {
        self.timing
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn elapsed(&self) -> Duration {
        self.base.elapsed()
    }

    /// The source frame being heard at `now` (since the stream's base), latency included.
    pub fn position_at(&self, now: Duration) -> f64 {
        // A seek is applied by the next callback (up to one buffer away). Until then the position
        // already is the target: otherwise a reader in that window sees the old place again and the
        // playhead flickers back (seen live, under load, as a paused seek that read the old frame).
        let pending = self.seek.load(Ordering::SeqCst);
        if pending >= 0 {
            return pending as f64;
        }
        let timing = *self.lock();
        timing.position(now, self.source_rate, self.total_frames as f64)
    }

    pub fn position(&self) -> f64 {
        self.position_at(self.elapsed())
    }

    /// True once the last frame has been heard.
    pub fn finished(&self) -> bool {
        self.finished_at(self.elapsed())
    }

    pub fn finished_at(&self, now: Duration) -> bool {
        let timing = *self.lock();
        timing
            .ended_at
            .is_some_and(|ended| now >= ended + timing.latency)
    }

    pub fn stats(&self) -> Stats {
        let timing = *self.lock();
        Stats {
            callbacks: self.callbacks.load(Ordering::Relaxed),
            late_callbacks: self.late_callbacks.load(Ordering::Relaxed),
            device_underruns: self.device_underruns.load(Ordering::Relaxed),
            frames_out: self.frames_out.load(Ordering::Relaxed),
            max_callback_gap: Duration::from_micros(self.max_gap_us.load(Ordering::Relaxed)),
            buffer_frames: self.buffer_frames.load(Ordering::Relaxed),
            latency: timing.latency,
        }
    }
}

impl Timing {
    fn position(&self, now: Duration, rate: u32, total: f64) -> f64 {
        if !self.seen {
            return self.held;
        }
        if !self.running {
            return self.held;
        }
        let heard_from = self.at + self.latency;
        let delta = now.as_secs_f64() - heard_from.as_secs_f64();
        let frame = self.source_frame + delta * f64::from(rate);
        frame.max(self.floor).min(total)
    }
}

/// What the callbacks measured, for a report.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    pub callbacks: u64,
    /// Callbacks that came more than 1.75 periods after the previous one.
    pub late_callbacks: u64,
    /// `BufferUnderrun` errors the device reported.
    pub device_underruns: u64,
    pub frames_out: u64,
    pub max_callback_gap: Duration,
    pub buffer_frames: u64,
    pub latency: Duration,
}

impl Stats {
    pub fn underruns(&self) -> u64 {
        self.late_callbacks + self.device_underruns
    }
}

/// Fills device buffers from a [`Source`]; owned by the data callback.
pub struct Engine {
    source: Source,
    shared: Arc<Shared>,
    out_rate: u32,
    channels: usize,
    /// Source position of the next output frame (fractional when resampling).
    cursor: f64,
    /// When the previous callback ran, to measure the gap between two.
    previous_at: Option<Duration>,
    exhausted: bool,
    /// The next playing callback starts (or restarts) the clock: the first, after a seek, after a pause.
    restart: bool,
}

/// What the device tells the data callback about one call.
#[derive(Clone, Copy, Debug)]
pub struct CallbackInfo {
    /// When the callback ran, since [`Shared::base`].
    pub at: Duration,
    /// How long after `at` the first frame of the buffer reaches the speaker.
    pub latency: Duration,
}

impl Engine {
    pub fn new(source: Source, shared: Arc<Shared>, out_rate: u32, channels: usize) -> Self {
        let cursor = shared.start_frame as f64;
        Self {
            source,
            shared,
            out_rate,
            channels: channels.max(1),
            cursor,
            previous_at: None,
            exhausted: false,
            restart: true,
        }
    }

    pub fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }

    /// Writes one buffer of interleaved samples (`channels` per frame).
    pub fn fill(&mut self, out: &mut [f32], info: CallbackInfo) {
        let frames = out.len() / self.channels;
        self.shared.callbacks.fetch_add(1, Ordering::Relaxed);
        self.shared
            .buffer_frames
            .store(frames as u64, Ordering::Relaxed);
        self.measure_gap(info.at, frames);

        let seek = self.shared.seek.swap(-1, Ordering::SeqCst);
        let seeked = seek >= 0;
        if seeked {
            self.cursor = seek as f64;
            self.exhausted = false;
            self.restart = true;
        }
        let paused = self.shared.paused.load(Ordering::SeqCst);

        let first = self.cursor;
        let written = if paused {
            out.fill(0.0);
            self.restart = true;
            0
        } else {
            let written = self.write(out);
            self.shared
                .frames_out
                .fetch_add(written as u64, Ordering::Relaxed);
            written
        };

        // A reader holding the lock costs this callback its timing update, never a wait.
        let Ok(mut timing) = self.shared.timing.try_lock() else {
            return;
        };
        let rate = self.shared.source_rate;
        let total = self.shared.total_frames as f64;
        if seeked {
            timing.ended_at = None;
        }
        if paused {
            if seeked {
                timing.held = first;
            } else if timing.running {
                // Hold what the listener hears right now, and resume from exactly there (the
                // `latency` worth of audio already written is dropped, not played twice).
                timing.held = timing.position(info.at, rate, total);
                self.cursor = timing.held;
            }
            timing.running = false;
            timing.seen = true;
            timing.floor = timing.held;
            return;
        }
        timing.source_frame = first;
        timing.at = info.at;
        timing.latency = info.latency;
        timing.running = true;
        timing.seen = true;
        if self.restart {
            // The start, a seek or a resume: older buffers are still being heard for `latency`,
            // but the position must not read below where playback was put.
            timing.floor = if seeked { first } else { timing.held };
            self.restart = false;
        }
        if self.exhausted && timing.ended_at.is_none() {
            // The last real frame was `written` frames into this buffer.
            let tail = Duration::from_secs_f64(written as f64 / f64::from(self.out_rate));
            timing.ended_at = Some(info.at + tail);
        }
    }

    fn measure_gap(&mut self, at: Duration, frames: usize) {
        if let Some(previous) = self.previous_at {
            let gap = at.saturating_sub(previous);
            let period = frames as f64 / f64::from(self.out_rate);
            self.shared
                .max_gap_us
                .fetch_max(gap.as_micros() as u64, Ordering::Relaxed);
            if period > 0.0 && gap.as_secs_f64() > period * LATE_FACTOR {
                self.shared.late_callbacks.fetch_add(1, Ordering::Relaxed);
            }
        }
        self.previous_at = Some(at);
    }

    /// Resamples (linearly) from the source into `out`, mapping stereo onto `channels`.
    fn write(&mut self, out: &mut [f32]) -> usize {
        let step = f64::from(self.source.sample_rate) / f64::from(self.out_rate);
        let total = self.source.frames() as f64;
        let samples = &self.source.samples;
        let mut written = 0;
        for frame in out.chunks_exact_mut(self.channels) {
            if self.cursor >= total {
                self.exhausted = true;
                frame.fill(0.0);
                continue;
            }
            let index = self.cursor as usize;
            let fraction = (self.cursor - index as f64) as f32;
            let next = (index + 1).min(self.source.frames() as usize - 1);
            let channel = |side: usize| {
                let a = samples[index * 2 + side];
                let b = samples[next * 2 + side];
                a + (b - a) * fraction
            };
            let (left, right) = (channel(0), channel(1));
            match self.channels {
                1 => frame[0] = (left + right) * 0.5,
                _ => {
                    frame[0] = left;
                    frame[1] = right;
                    frame[2..].fill(0.0);
                }
            }
            self.cursor += step;
            written += 1;
        }
        written
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    /// A ramp: stereo frame `n` is `(n, -n)` scaled, so a sample says which source frame it is.
    fn ramp(seconds: f64) -> Source {
        let frames = (seconds * f64::from(RATE)) as usize;
        let mut samples = Vec::with_capacity(frames * 2);
        for n in 0..frames {
            samples.push(n as f32 / 1_000_000.0);
            samples.push(-(n as f32) / 1_000_000.0);
        }
        Source {
            samples: samples.into(),
            sample_rate: RATE,
        }
    }

    fn engine(source: Source, start: u64, out_rate: u32, channels: usize) -> Engine {
        let shared = Arc::new(Shared::new(&source, start));
        Engine::new(source, shared, out_rate, channels)
    }

    fn info(at_ms: u64, latency_ms: u64) -> CallbackInfo {
        CallbackInfo {
            at: Duration::from_millis(at_ms),
            latency: Duration::from_millis(latency_ms),
        }
    }

    fn frame_of(sample: f32) -> f64 {
        f64::from(sample) * 1_000_000.0
    }

    #[test]
    fn a_buffer_carries_consecutive_source_frames_in_both_channels() {
        let mut engine = engine(ramp(1.0), 0, RATE, 2);
        let mut buffer = vec![0.0; 480 * 2];
        engine.fill(&mut buffer, info(0, 10));
        assert_eq!(frame_of(buffer[0]), 0.0);
        assert_eq!(frame_of(buffer[2]).round(), 1.0);
        assert_eq!(frame_of(buffer[2 * 479]).round(), 479.0);
        assert_eq!(buffer[3], -buffer[2], "right channel is the ramp's mirror");
        engine.fill(&mut buffer, info(10, 10));
        assert_eq!(
            frame_of(buffer[0]).round(),
            480.0,
            "the next call continues, with no gap"
        );
    }

    #[test]
    fn the_heard_position_trails_the_written_one_by_the_latency() {
        let mut engine = engine(ramp(2.0), 0, RATE, 2);
        let mut buffer = vec![0.0; 480 * 2];
        // Callbacks every 10 ms with 50 ms of latency: at t = 100 ms the speaker plays what was
        // written at t = 50 ms, source frame 2400.
        for n in 0..11 {
            engine.fill(&mut buffer, info(n * 10, 50));
        }
        let shared = engine.shared().clone();
        let heard = shared.position_at(Duration::from_millis(100));
        assert!((heard - 2_400.0).abs() < 1.0, "heard {heard}");
        // Before the first buffer arrives the position is the start, never negative.
        assert_eq!(shared.position_at(Duration::from_millis(1)), 0.0);
        // Extrapolating 5 ms past the last callback adds 240 frames: it is a clock, not a guess.
        let later = shared.position_at(Duration::from_millis(105));
        assert!((later - heard - 240.0).abs() < 1.0, "{later} vs {heard}");
    }

    #[test]
    fn pause_holds_the_heard_position_and_resume_continues_from_it() {
        let mut engine = engine(ramp(2.0), 0, RATE, 2);
        let mut buffer = vec![0.0; 480 * 2];
        for n in 0..20 {
            engine.fill(&mut buffer, info(n * 10, 30));
        }
        let shared = engine.shared().clone();
        shared.pause();
        engine.fill(&mut buffer, info(200, 30));
        assert!(
            buffer.iter().all(|sample| *sample == 0.0),
            "paused is silence"
        );
        let held = shared.position_at(Duration::from_millis(200));
        let later = shared.position_at(Duration::from_millis(900));
        assert_eq!(held, later, "the position does not move while paused");
        // 170 ms of what was written has been heard (200 ms − 30 ms latency).
        assert!((held - 170.0 * 48.0).abs() < 2.0, "held {held}");
        shared.resume();
        engine.fill(&mut buffer, info(1_000, 30));
        assert!(
            (frame_of(buffer[0]) - held).abs() < 1.0,
            "resume continues from the frame that was being heard, {held}"
        );
    }

    #[test]
    fn a_seek_lands_on_its_frame_and_the_position_never_reads_before_it() {
        let mut engine = engine(ramp(4.0), 0, RATE, 2);
        let mut buffer = vec![0.0; 480 * 2];
        for n in 0..10 {
            engine.fill(&mut buffer, info(n * 10, 40));
        }
        let shared = engine.shared().clone();
        shared.seek(96_000);
        engine.fill(&mut buffer, info(100, 40));
        assert_eq!(frame_of(buffer[0]).round(), 96_000.0);
        // Still within the latency: the old audio is being heard, but the position already says 2 s.
        assert!(shared.position_at(Duration::from_millis(110)) >= 96_000.0);
        let settled = shared.position_at(Duration::from_millis(160));
        assert!(
            (settled - (96_000.0 + 20.0 * 48.0)).abs() < 2.0,
            "{settled}"
        );
        // A seek past the end is the end.
        shared.seek(10_000_000);
        engine.fill(&mut buffer, info(110, 40));
        assert!(buffer.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn a_seek_is_the_position_at_once_even_before_the_next_callback_and_while_paused() {
        let mut engine = engine(ramp(4.0), 0, RATE, 2);
        let mut buffer = vec![0.0; 480 * 2];
        for n in 0..10 {
            engine.fill(&mut buffer, info(n * 10, 40));
        }
        let shared = engine.shared().clone();
        shared.pause();
        engine.fill(&mut buffer, info(100, 40));
        shared.seek(72_000);
        // No callback has run since the seek: the old position must not leak back.
        assert_eq!(shared.position_at(Duration::from_millis(101)), 72_000.0);
        engine.fill(&mut buffer, info(110, 40));
        assert_eq!(shared.position_at(Duration::from_millis(120)), 72_000.0);
        assert_eq!(shared.position_at(Duration::from_millis(900)), 72_000.0);
    }

    #[test]
    fn the_end_is_reached_one_latency_after_the_last_frame_was_written() {
        let mut engine = engine(ramp(0.05), 0, RATE, 2); // 2 400 frames
        let mut buffer = vec![0.0; 1_000 * 2];
        let shared = engine.shared().clone();
        for n in 0..3 {
            engine.fill(&mut buffer, info(n * 20, 30));
        }
        assert!(
            !shared.finished_at(Duration::from_millis(50)),
            "still audible"
        );
        // Last real frame written 400 frames (8.3 ms) into the 40 ms callback.
        assert!(!shared.finished_at(Duration::from_millis(70)));
        assert!(shared.finished_at(Duration::from_millis(80)));
        assert!(shared.position_at(Duration::from_millis(500)) <= 2_400.0);
        engine.fill(&mut buffer, info(60, 30));
        assert!(
            buffer.iter().all(|sample| *sample == 0.0),
            "after the end: silence"
        );
    }

    #[test]
    fn a_device_at_another_rate_gets_a_resampled_but_time_true_signal() {
        let mut engine = engine(ramp(1.0), 0, 44_100, 2);
        let mut buffer = vec![0.0; 441 * 2]; // 10 ms at 44.1 kHz
        engine.fill(&mut buffer, info(0, 20));
        engine.fill(&mut buffer, info(10, 20));
        // 10 ms later the source cursor is 480 frames further, not 441.
        assert!(
            (frame_of(buffer[0]) - 480.0).abs() < 1.0,
            "{}",
            frame_of(buffer[0])
        );
        let heard = engine.shared().position_at(Duration::from_millis(40));
        assert!((heard - 20.0 * 48.0).abs() < 2.0, "{heard}");
    }

    #[test]
    fn mono_devices_get_the_mixdown_and_wide_ones_get_silence_on_the_extra_channels() {
        let mut mono = engine(ramp(0.1), 0, RATE, 1);
        let mut buffer = vec![0.0; 64];
        mono.fill(&mut buffer, info(0, 0));
        assert!(
            buffer.iter().all(|sample| sample.abs() < 1e-9),
            "(n − n) / 2 is silence"
        );
        let mut wide = engine(ramp(0.1), 100, RATE, 6);
        let mut buffer = vec![9.0; 6 * 8];
        wide.fill(&mut buffer, info(0, 0));
        assert_eq!(frame_of(buffer[0]).round(), 100.0);
        assert!(
            buffer
                .chunks(6)
                .all(|frame| frame[2..].iter().all(|s| *s == 0.0))
        );
    }

    #[test]
    fn a_late_callback_is_counted_as_an_underrun_suspect() {
        let mut engine = engine(ramp(1.0), 0, RATE, 2);
        let mut buffer = vec![0.0; 480 * 2]; // 10 ms period
        engine.fill(&mut buffer, info(0, 10));
        engine.fill(&mut buffer, info(10, 10));
        engine.fill(&mut buffer, info(21, 10)); // 11 ms: on time
        engine.fill(&mut buffer, info(60, 10)); // 39 ms: three periods late
        let stats = engine.shared().stats();
        assert_eq!(stats.callbacks, 4);
        assert_eq!(stats.late_callbacks, 1);
        assert_eq!(stats.max_callback_gap, Duration::from_millis(39));
        assert_eq!(stats.latency, Duration::from_millis(10));
        engine.shared().device_underrun();
        assert_eq!(engine.shared().stats().underruns(), 2);
    }

    #[test]
    fn starting_in_the_middle_reports_that_frame_before_anything_is_heard() {
        let engine = engine(ramp(2.0), 24_000, RATE, 2);
        assert_eq!(
            engine.shared().position_at(Duration::from_secs(5)),
            24_000.0
        );
    }
}
