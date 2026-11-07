//! Interactive playback of the sequence: a clock that moves the playhead, the viewport that
//! follows it, and the device audio that plays the same mix.
//!
//! What is real here and what is not:
//! - The sound is the core's `AudioPlan` of the *whole* sequence (the plan export uses), mixed off
//!   the UI thread and handed to `tessel-editor-audio`, which plays it on the default output
//!   device through CoreAudio (WASAPI/ALSA elsewhere). Pause, resume and seek are rules of that
//!   crate's sample clock; nothing here drains, sleeps or waits for a process.
//! - While a device plays, **the sample clock is the clock**: each tick asks the device what is
//!   being heard (callback timestamps and the host's latency estimate included) and seeks the
//!   playhead to that frame. The picture is the viewport's normal frame path: the host renders the
//!   newest requested frame and drops stale ones, so the monitor never falls behind the clock.
//! - Without a usable device (none present, cannot be configured, no decoded audio to mix) the
//!   report says so with the error (`audio.error.code`, e.g. `audio_output_unavailable`) and the
//!   picture runs on the wall clock (`clock: "wall"`), silent. It never pretends to be in sync.
//! - What is *not* known: the acoustic delay after the host's estimate (a Bluetooth headset adds
//!   far more than the buffer the host can see), and whether anyone heard anything. The numbers
//!   below are technical measurements of the clock, callbacks and underruns, nothing more.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::*;
use serde_json::{Value, json};
use tessel_editor_audio::engine::Shared;
use tessel_editor_audio::{AudioError, DeviceInfo, Output, Source, Status};
use tessel_editor_core::audio::{AudioBackend, OfflineMixer};
use tessel_editor_core::state::Command;
use tessel_editor_core::{CommandError, EditorState, Raw};

use super::{Activity, Source as Origin, Workstation};

/// The clock is polled this many times per frame; a delayed tick only costs skipped frames.
const TICKS_PER_FRAME: f64 = 2.0;

/// The mix is held in memory while it plays (384 KB per second); longer sequences are refused
/// with an explicit error and play silent instead of exhausting memory.
const MAX_MIX_SECONDS: f64 = 30.0 * 60.0;

/// Where the sound of this playback stands.
enum Audio {
    /// The device is being opened on the audio thread; the clock has not started.
    Opening(Output, Instant),
    Running {
        output: Output,
        device: DeviceInfo,
        open: Duration,
    },
    /// No sound: the reason is part of every report.
    Silent(AudioError),
}

/// The picture's clock when no device provides one.
struct WallClock {
    anchor: Instant,
    base: f64,
    held: Option<f64>,
}

impl WallClock {
    fn frame(&self, fps: f64) -> f64 {
        self.held
            .unwrap_or_else(|| self.base + self.anchor.elapsed().as_secs_f64() * fps)
    }

    fn pause(&mut self, fps: f64) {
        if self.held.is_none() {
            self.held = Some(self.frame(fps));
        }
    }

    fn resume(&mut self) {
        if let Some(held) = self.held.take() {
            self.anchor = Instant::now();
            self.base = held;
        }
    }

    fn seek(&mut self, frame: f64) {
        self.base = frame;
        self.anchor = Instant::now();
        if self.held.is_some() {
            self.held = Some(frame);
        }
    }
}

pub struct Playback {
    anchor: Instant,
    from: i64,
    fps: f64,
    audio: Audio,
    wall: WallClock,
    paused: bool,
    /// What the clock asked for last, so a tick only seeks when the frame changed.
    frame: i64,
    ticks: u64,
    /// Frames the clock jumped over between two ticks (late ticks): skipped, not rendered.
    skipped: u64,
    max_gap: i64,
}

/// Mix of the sequence, being prepared off the UI thread.
pub struct Preparing {
    pub from: i64,
}

#[derive(Default)]
pub struct PlaybackState {
    pub preparing: Option<Preparing>,
    pub active: Option<Playback>,
    last: Option<Value>,
}

/// The core's audio plan of the whole sequence, mixed to the stereo samples the device plays.
/// The same `plan_audio` an export uses, so what is heard is what is exported.
fn mix_sequence(state: &EditorState) -> Result<Source, CommandError> {
    let length = state.timeline.length;
    let fps = f64::from(state.timeline.rate.num()) / f64::from(state.timeline.rate.den());
    if length as f64 / fps > MAX_MIX_SECONDS {
        return Err(CommandError::new(
            "audio_range_too_long",
            format!(
                "The sequence is longer than {} minutes; its sound is not played (the picture is)",
                MAX_MIX_SECONDS / 60.0
            ),
        ));
    }
    let plan = state.plan_audio(0, length)?;
    let pcm = OfflineMixer::default()
        .mix(&plan)
        .map_err(|error| CommandError::new("audio_mix_failed", error.message))?;
    Ok(Source {
        samples: Arc::from(pcm.samples),
        sample_rate: pcm.sample_rate,
    })
}

impl Playback {
    fn shared(&self) -> Option<&Arc<Shared>> {
        match &self.audio {
            Audio::Opening(output, _) | Audio::Running { output, .. } => Some(output.shared()),
            Audio::Silent(_) => None,
        }
    }

    /// Opening → running (or silent, explicitly) once the audio thread has an answer. Returns the
    /// error when the device could not be used, so the caller can say why there is no sound.
    fn settle(&mut self) -> Option<AudioError> {
        let Audio::Opening(output, since) = &self.audio else {
            return None;
        };
        match output.status() {
            Status::Opening => None,
            Status::Playing(device) => {
                let open = since.elapsed();
                let Audio::Opening(output, _) = std::mem::replace(
                    &mut self.audio,
                    Audio::Silent(AudioError {
                        code: "audio_output_unavailable",
                        message: String::new(),
                    }),
                ) else {
                    return None;
                };
                self.audio = Audio::Running {
                    output,
                    device,
                    open,
                };
                None
            }
            Status::Failed(error) => {
                // The picture starts from where it was, on the wall clock.
                self.wall = WallClock {
                    anchor: Instant::now(),
                    base: self.frame as f64,
                    held: self.paused.then_some(self.frame as f64),
                };
                self.audio = Audio::Silent(error.clone());
                Some(error)
            }
        }
    }

    /// The timeline frame the clock says is current, `None` while the device is still opening.
    fn clock_frame(&self, rate: u32) -> Option<f64> {
        match &self.audio {
            Audio::Opening(..) => None,
            Audio::Running { output, .. } => {
                Some(output.shared().position() / f64::from(rate) * self.fps)
            }
            Audio::Silent(_) => Some(self.wall.frame(self.fps)),
        }
    }

    fn clock_name(&self) -> &'static str {
        match &self.audio {
            Audio::Opening(..) => "starting",
            Audio::Running { .. } => "audio_device",
            Audio::Silent(_) => "wall",
        }
    }

    fn audio_report(&self) -> Value {
        match &self.audio {
            Audio::Opening(..) => json!({ "output": null, "status": "opening" }),
            Audio::Silent(error) => json!({ "output": null,
                "error": { "code": error.code, "message": error.message } }),
            Audio::Running {
                output,
                device,
                open,
            } => {
                let shared = output.shared();
                let stats = shared.stats();
                json!({
                    "output": device.to_json(),
                    "open_ms": open.as_secs_f64() * 1000.0,
                    // The host's own estimate (CoreAudio: from the device buffer); acoustic and
                    // Bluetooth delay beyond it are not observable.
                    "latency_ms": stats.latency.as_secs_f64() * 1000.0,
                    "buffer_frames": stats.buffer_frames,
                    "callbacks": stats.callbacks,
                    "underruns": stats.underruns(),
                    "late_callbacks": stats.late_callbacks,
                    "device_underruns": stats.device_underruns,
                    "max_callback_gap_ms": stats.max_callback_gap.as_secs_f64() * 1000.0,
                    "position_frames": shared.position(),
                    "paused": shared.is_paused(),
                    "finished": shared.finished(),
                    "messages": output.messages(),
                    "heard": false,
                })
            }
        }
    }
}

impl PlaybackState {
    /// The frame of a clock that is *running*: `None` while paused, stopped or preparing. The
    /// debug meter starts from this: a paused playback expects no frames.
    pub fn running_frame(&self) -> Option<i64> {
        self.active
            .as_ref()
            .filter(|active| !active.paused)
            .map(|active| active.frame)
    }

    pub fn is_playing(&self) -> bool {
        self.active.is_some() || self.preparing.is_some()
    }

    pub fn report(&self) -> Value {
        if let Some(preparing) = &self.preparing {
            return json!({ "state": "preparing", "from": preparing.from });
        }
        let Some(active) = &self.active else {
            return json!({ "state": "stopped", "last": self.last });
        };
        let elapsed = active.anchor.elapsed();
        let rate = active
            .shared()
            .map_or(48_000, |shared| shared.sample_rate());
        let clock = active
            .clock_frame(rate)
            .map_or(active.frame, |frame| frame.floor() as i64);
        json!({
            "state": if active.paused { "paused" } else { "playing" },
            "from": active.from, "frame": active.frame, "fps": active.fps,
            "clock": active.clock_name(),
            "elapsed_ms": elapsed.as_secs_f64() * 1000.0,
            // Frame the clock says we are at, minus the one the playhead shows: 0 or 1 when
            // keeping up.
            "lag_frames": clock - active.frame,
            "ticks": active.ticks, "skipped_frames": active.skipped, "max_gap_frames": active.max_gap,
            "audio": active.audio_report(),
        })
    }
}

impl Workstation {
    /// Starts at `from` (default: the playhead, or the start when it is at the end).
    pub fn playback_start(
        &mut self,
        from: Option<i64>,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        if self.playback.is_playing() {
            return Err(CommandError::new(
                "already_playing",
                "Playback is already running",
            ));
        }
        let length = self.state.timeline.length;
        let playhead = self.state.timeline.playhead;
        let from = from.unwrap_or(if playhead >= length { 0 } else { playhead });
        if !(0..length).contains(&from) {
            return Err(CommandError::new(
                "out_of_bounds",
                format!("Playback must start inside 0..{length}"),
            ));
        }
        self.playback.preparing = Some(Preparing { from });
        let snapshot = self.state.clone();
        cx.spawn(async move |this, cx| {
            // The mix is offline and can take a moment on a long sequence: never on the UI thread.
            let mixed = cx
                .background_executor()
                .spawn(async move { mix_sequence(&snapshot) })
                .await;
            let _ = this.update(cx, |this, cx| this.playback_begin(from, mixed, cx));
        })
        .detach();
        cx.notify();
        Ok(json!({ "action": "playback.start", "from": from, "state": "preparing" }))
    }

    pub(super) fn playback_begin(
        &mut self,
        from: i64,
        mixed: Result<Source, CommandError>,
        cx: &mut Context<Self>,
    ) {
        if self.playback.preparing.is_none() {
            // Stopped while the mix was being made.
            return;
        }
        self.playback.preparing = None;
        let rate = self.state.timeline.rate;
        let fps = f64::from(rate.num()) / f64::from(rate.den());
        let audio = match mixed {
            Ok(source) => {
                // The mix covers the whole sequence from frame 0, so a seek never needs another one.
                let start = (from as f64 / fps * f64::from(source.sample_rate)) as u64;
                Audio::Opening(Output::start(source, start), Instant::now())
            }
            Err(error) => Audio::Silent(AudioError {
                code: error.code,
                message: error.message,
            }),
        };
        let silent = matches!(audio, Audio::Silent(_));
        self.playback.active = Some(Playback {
            anchor: Instant::now(),
            from,
            fps,
            audio,
            wall: WallClock {
                anchor: Instant::now(),
                base: from as f64,
                held: None,
            },
            paused: false,
            frame: from,
            ticks: 0,
            skipped: 0,
            max_gap: 0,
        });
        self.perf_playback_started(from);
        let interval = Duration::from_secs_f64(1.0 / (fps * TICKS_PER_FRAME));
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(interval).await;
                let keep_going = this
                    .update(cx, |this, cx| this.playback_tick(cx))
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        })
        .detach();
        self.activity = Some(Activity {
            source: Origin::Gesture,
            text: if silent {
                "Reprodução (sem som)"
            } else {
                "Reprodução"
            }
            .into(),
            error: silent,
        });
        cx.notify();
    }

    /// One clock tick: moves the playhead to the frame the clock says it is. Returns whether to
    /// keep ticking.
    pub(super) fn playback_tick(&mut self, cx: &mut Context<Self>) -> bool {
        let length = self.state.timeline.length;
        let Some(active) = &mut self.playback.active else {
            return false;
        };
        if let Some(error) = active.settle() {
            // The device could not be used: the picture goes on silent, and the status bar says why.
            self.activity = Some(Activity {
                source: Origin::Gesture,
                text: format!("Sem som: {}", error.message),
                error: true,
            });
        }
        active.ticks += 1;
        let rate = active
            .shared()
            .map_or(48_000, |shared| shared.sample_rate());
        let Some(clock) = active.clock_frame(rate) else {
            // Waiting for the device to open: the playhead stays where playback was asked to start.
            cx.notify();
            return true;
        };
        let frame = (clock.floor() as i64).max(0);
        let ended = match &active.audio {
            Audio::Running { output, .. } => output.shared().finished(),
            _ => false,
        } || frame >= length;
        if ended {
            self.playback_stop(cx);
            let _ = self.state.execute(Command::SetPlayhead {
                frame: Raw::Number(length as f64),
            });
            self.viewport_sync(cx);
            return false;
        }
        let gap = frame - active.frame;
        if gap > 1 {
            active.skipped += (gap - 1) as u64;
        }
        active.max_gap = active.max_gap.max(gap);
        if frame != active.frame {
            active.frame = frame;
            self.perf_playback_clock(frame);
            // A plain playhead move: the same command as a click on the ruler, so the
            // monitor, the timeline and the inspector follow it with no special case.
            let _ = self.state.execute(Command::SetPlayhead {
                frame: Raw::Number(frame as f64),
            });
            self.viewport_sync(cx);
        }
        cx.notify();
        true
    }

    pub fn playback_stop(&mut self, cx: &mut Context<Self>) -> Value {
        self.playback.preparing = None;
        let mut report = self.playback.report();
        self.perf_playback_stopped();
        if let Some(mut active) = self.playback.active.take() {
            // Asks the audio thread to release the device; it does that off this thread.
            match &mut active.audio {
                Audio::Opening(output, _) | Audio::Running { output, .. } => output.close(),
                Audio::Silent(_) => {}
            }
            report["state"] = json!("stopped");
            self.playback.last = Some(report.clone());
        }
        cx.notify();
        json!({ "action": "playback.stop", "report": report })
    }

    pub fn playback_toggle(&mut self, cx: &mut Context<Self>) -> Result<Value, CommandError> {
        if self.playback.is_playing() {
            return Ok(self.playback_stop(cx));
        }
        self.playback_start(None, cx)
    }

    fn active_playback(&mut self) -> Result<&mut Playback, CommandError> {
        self.playback.active.as_mut().ok_or_else(|| {
            CommandError::new(
                "not_playing",
                "Playback is not running (`playback.start` first)",
            )
        })
    }

    /// Holds the picture and the sound where they are; `playback.resume` continues from there.
    pub fn playback_pause(&mut self, cx: &mut Context<Self>) -> Result<Value, CommandError> {
        let active = self.active_playback()?;
        if let Some(shared) = active.shared() {
            shared.pause();
        }
        let fps = active.fps;
        active.wall.pause(fps);
        active.paused = true;
        // Nothing is expected while paused: silence is a rest, not a stall (F049).
        self.perf_playback_stopped();
        cx.notify();
        Ok(json!({ "action": "playback.pause", "report": self.playback.report() }))
    }

    pub fn playback_resume(&mut self, cx: &mut Context<Self>) -> Result<Value, CommandError> {
        let active = self.active_playback()?;
        if let Some(shared) = active.shared() {
            shared.resume();
        }
        active.wall.resume();
        active.paused = false;
        let frame = active.frame;
        // The run restarts from the frame on screen; the pause is not a gap (F049). The meter
        // keeps the counts of a playback that never paused (`Meter::playback_started`).
        self.perf_playback_started(frame);
        cx.notify();
        Ok(json!({ "action": "playback.resume", "report": self.playback.report() }))
    }

    /// Moves picture and sound to `frame` without stopping (paused stays paused).
    pub fn playback_seek(
        &mut self,
        frame: i64,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        let length = self.state.timeline.length;
        if !(0..length).contains(&frame) {
            return Err(CommandError::new(
                "out_of_bounds",
                format!("Seek must land inside 0..{length}"),
            ));
        }
        let active = self.active_playback()?;
        if let Some(shared) = active.shared() {
            let audio_frame = frame as f64 / active.fps * f64::from(shared.sample_rate());
            shared.seek(audio_frame as u64);
        }
        active.wall.seek(frame as f64);
        active.frame = frame;
        self.perf_playback_seeked(frame);
        let _ = self.state.execute(Command::SetPlayhead {
            frame: Raw::Number(frame as f64),
        });
        self.viewport_sync(cx);
        cx.notify();
        Ok(json!({ "action": "playback.seek", "frame": frame, "report": self.playback.report() }))
    }

    /// `playback.*` as operations, for scripts and tests.
    pub(super) fn run_playback_op(
        &mut self,
        request: &Value,
        cx: &mut Context<Self>,
    ) -> Option<Result<Value, CommandError>> {
        let result = match request["op"].as_str()? {
            "playback.start" => self.playback_start(request["from"].as_i64(), cx),
            "playback.stop" => Ok(self.playback_stop(cx)),
            "playback.pause" => self.playback_pause(cx),
            "playback.resume" => self.playback_resume(cx),
            "playback.seek" => match request["frame"].as_i64() {
                Some(frame) => self.playback_seek(frame, cx),
                None => Err(CommandError::new(
                    "invalid_request",
                    "`playback.seek` needs an integer `frame`",
                )),
            },
            "playback.status" => Ok(self.playback.report()),
            _ => return None,
        };
        Some(result)
    }
}
